"""Version-guarded FastAPI ↔ Starlette compatibility adapter.

Problem this solves
-------------------
FastAPI 0.115.x still forwards the long-deprecated ``on_startup`` /
``on_shutdown`` keyword arguments into ``starlette.routing.Router.__init__``
(``fastapi/routing.py`` → ``super().__init__(..., on_startup=..., ...)``).
Starlette 1.0 **removed** those parameters (they were replaced by ``lifespan``),
so on an environment where ``pip`` resolved ``fastapi==0.115.*`` together with
``starlette>=1.0``, constructing *any* ``FastAPI(...)`` app dies with::

    TypeError: Router.__init__() got an unexpected keyword argument 'on_startup'

and there is no compatible pair available offline (no PyPI access).

Fix
---
:meth:`install` wraps ``starlette.routing.Router.__init__`` **only when the
installed Starlette no longer declares those parameters**, translating the
legacy kwargs into an equivalent async lifespan context (startups run in order
on enter, shutdowns run in reverse order on exit).  On Starlette versions that
still accept the kwargs the call is an immediate no-op passthrough, so the shim
is safe to install unconditionally at import time of :mod:`marguerite_workbench.api`.

Design notes (Bloom gates)
--------------------------
* Version-guarded: introspects the installed signature instead of guessing a
  version string — works for any Starlette, past or future (R5: no magic).
* Honest failures (R3): passing *both* a real lifespan and non-empty legacy
  handlers is ambiguous and raises a clear ``TypeError``; nothing is silently
  dropped without a log record.
* Idempotent: repeated installs are detected and skipped.
"""

from __future__ import annotations

import inspect
import logging
from contextlib import asynccontextmanager
from typing import Any, Callable, List, Optional

logger = logging.getLogger("marguerite.compat")

_LEGACY_KWARGS = ("on_startup", "on_shutdown")
_INSTALLED_FLAG = "_marguerite_legacy_lifespan_compat"


def _starlette_router_needs_shim() -> bool:
    """True when the installed ``starlette.routing.Router`` lacks the legacy kwargs."""
    from starlette.routing import Router

    return "on_startup" not in inspect.signature(Router.__init__).parameters


def install() -> bool:
    """Install the adapters when needed. Returns True if any wrap was applied."""
    from starlette.routing import Router

    applied = False

    if not getattr(Router, _INSTALLED_FLAG, False) and _starlette_router_needs_shim():
        applied |= _wrap_router_init(Router)

    # Self-healing middleware stack: Starlette >=1.0 reads ``max_body_size``
    # while building middleware, but FastAPI 0.115 never chains into
    # ``Starlette.__init__`` so the attribute may be missing on *any* app
    # instance.  Backfilling here fixes every construction site at once.
    from starlette.applications import Starlette

    if not getattr(Starlette, _INSTALLED_FLAG, False) and _starlette_stack_needs_guard():
        _wrap_middleware_stack(Starlette)
        applied = True

    return applied


def _wrap_router_init(router_cls: type) -> bool:
    original_init = router_cls.__init__

    def _compat_init(
        router_self: Any,
        *args: Any,
        on_startup: Optional[List[Callable[[Any], Any]]] = None,
        on_shutdown: Optional[List[Callable[[Any], Any]]] = None,
        **kwargs: Any,
    ) -> None:
        has_handlers = bool(on_startup or on_shutdown)
        lifespan = kwargs.get("lifespan")
        if has_handlers and lifespan is not None:
            raise TypeError(
                "Router received both a lifespan and legacy on_startup/on_shutdown "
                "handlers; the combined semantics are ambiguous — migrate the "
                "handlers into the lifespan context instead."
            )
        if has_handlers:
            @asynccontextmanager
            async def _translated(app: Any):  # pragma: no cover - trivial glue
                for handler in on_startup or []:
                    maybe = handler()
                    # Support both plain callables and coroutine functions.
                    if inspect.isawaitable(maybe):
                        await maybe
                try:
                    yield
                finally:
                    for handler in reversed(on_shutdown or []):
                        maybe = handler()
                        if inspect.isawaitable(maybe):
                            await maybe

            kwargs["lifespan"] = _translated
            logger.info(
                "translated %d startup / %d shutdown handler(s) into a lifespan "
                "(Starlette >=1.0 removed on_startup/on_shutdown)",
                len(on_startup or []),
                len(on_shutdown or []),
            )
        # Empty legacy lists are simply dropped — they carry no behaviour.
        return original_init(router_self, *args, **kwargs)

    router_cls.__init__ = _compat_init  # type: ignore[method-assign]
    setattr(router_cls, _INSTALLED_FLAG, True)
    logger.info(
        "fastapi/starlette lifecycle compat adapter installed "
        "(starlette Router lacks %s)",
        "/".join(_LEGACY_KWARGS),
    )
    return True


def _starlette_stack_needs_guard() -> bool:
    """True when the installed build_middleware_stack references max_body_size."""
    import inspect as inspect_mod

    from starlette.applications import Starlette

    src = inspect_mod.getsource(Starlette.build_middleware_stack)
    return "max_body_size" in src


def _wrap_middleware_stack(starlette_cls: type) -> None:
    original_build = starlette_cls.build_middleware_stack

    def _guarded_build(stack_self: Any) -> Any:
        if not hasattr(stack_self, "max_body_size"):
            # ``None`` == unlimited, matching the historical default of every
            # Starlette/FastAPI release this workbench ever targeted.
            stack_self.max_body_size = None
        return original_build(stack_self)

    starlette_cls.build_middleware_stack = _guarded_build  # type: ignore[method-assign]
    setattr(starlette_cls, _INSTALLED_FLAG, True)
    logger.info(
        "starlette>=1.0 middleware-stack guard installed "
        "(backfills max_body_size on legacy FastAPI instances)"
    )


_APP_ATTR_FLAG = "_marguerite_attrs_ensured"


def ensure_app_attributes(app: Any) -> None:
    """Backfill attributes Starlette >=1.0 expects but old FastAPI never sets.

    FastAPI 0.115 does not chain into ``Starlette.__init__`` (it wires its own
    state), so instance fields introduced later in Starlette — notably
    ``max_body_size``, consumed by ``build_middleware_stack()`` — are missing
    and explode on the first request.  Idempotent; logs what it backfills.
    """
    if getattr(app, _APP_ATTR_FLAG, False):
        return
    backfilled: list[str] = []
    if not hasattr(app, "max_body_size"):
        # ``None`` == unlimited, matching the historical default of every
        # Starlette/FastAPI release this workbench ever targeted.
        app.max_body_size = None
        backfilled.append("max_body_size=None")
    if backfilled:
        logger.info("backfilled starlette>=1.0 app attributes: %s", ", ".join(backfilled))
    setattr(app, _APP_ATTR_FLAG, True)
