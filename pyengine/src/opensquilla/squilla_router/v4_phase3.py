"""OpenSquilla adapter for the copied V4 Phase 3 model router bundle."""

from __future__ import annotations

import hashlib
import sys
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import structlog
import yaml

from opensquilla.router_runtime_diagnostics import (
    ROUTER_PYTHON_DEPENDENCY_MISSING,
    ROUTER_WEIGHTS_NOT_FETCHED,
    REPAIRABLE_ROUTER_DEGRADATIONS,
    classify_router_runtime_error,
    router_runtime_hint,
)
from opensquilla.router_tiers import (
    DEFAULT_TEXT_TIER,
    ROUTE_CLASS_TO_TIER,
)
from opensquilla.squilla_router.bundle_inspect import (
    REQUIRED_BUNDLE_FILES as _REQUIRED_BUNDLE_FILES,
)
from opensquilla.squilla_router.bundle_inspect import (
    ManifestScan,
    default_bundle_dir,
    inspect_bundle,
    read_bundle_version,
    scan_manifest,
)
from opensquilla.squilla_router.controller import TIER_ORDER, select_localized_prompt_hint

__all__ = [
    "ManifestScan",
    "V4Phase3Strategy",
    "default_bundle_dir",
    "inspect_bundle",
    "runtime_src_import_path",
    "scan_manifest",
]

log = structlog.get_logger(__name__)

_ROUTE_CLASS_TO_TIER: dict[str, str] = dict(ROUTE_CLASS_TO_TIER)


@contextmanager
def runtime_src_import_path(bundle_dir: Path) -> Iterator[None]:
    """Temporarily expose the copied bundle's ``runtime_src`` import root."""
    old_path = list(sys.path)
    sys.path.insert(0, str(bundle_dir / "runtime_src"))
    try:
        yield
    finally:
        sys.path[:] = old_path


def _find_valid_tier(start_tier: str, valid_tiers: list[str]) -> str:
    if not valid_tiers:
        return DEFAULT_TEXT_TIER
    start_idx = TIER_ORDER.index(start_tier) if start_tier in TIER_ORDER else 1
    for idx in range(start_idx, len(TIER_ORDER)):
        if TIER_ORDER[idx] in valid_tiers:
            return TIER_ORDER[idx]
    for tier in TIER_ORDER:
        if tier in valid_tiers:
            return tier
    return valid_tiers[0]


class V4Phase3Strategy:
    """History-aware strategy wrapping the delivered V4 Phase 3 inference core."""

    requires_history = True
    source = "v4_phase3"

    def __init__(
        self,
        bundle_dir: str | Path | None = None,
        confidence_threshold: float = 0.5,
        require_router_runtime: bool = False,
        use_aux_head: bool | None = None,
        emit_train_features: bool = False,
        emit_raw_bge: bool = False,
    ) -> None:
        self.bundle_dir = Path(bundle_dir) if bundle_dir else default_bundle_dir()
        self._threshold = confidence_threshold
        self._require_router_runtime = require_router_runtime
        self._emit_train_features = emit_train_features
        self._emit_raw_bge = emit_raw_bge
        self._core: Any | None = None
        self._request_type: Any | None = None
        self._config: dict[str, Any] = {}
        self._model_version = "unknown"
        self._feature_schema_version = "unknown"
        self._available = False
        self._degraded_warned = False
        self._init_error_kind: str | None = None
        self._init_error: str | None = None

        try:
            self._init_runtime(use_aux_head=use_aux_head)
        except Exception as exc:
            error_kind = classify_router_runtime_error(exc)
            self._init_error_kind = error_kind
            self._init_error = str(exc)
            # Git LFS pointers are not an initialization crash: the bundle
            # layout is intact and the bytes were simply never pulled. Logging
            # ``init_failed`` for them (as this used to) made a one-command
            # operator fix look like a broken installation.
            #
            # 2026-10-22：把同一判据推广到「Python 依赖缺失」（典型：装完
            # 桌面版后 `No module named 'lightgbm'`）。它与 LFS 未拉取是**同一类**：
            #   · 安装本身是完整的（RouterBundleDegraded 的 docstring 已写明
            #     "ML routing is an optional enhancement and the rule-based router
            #     still serves every turn"）；
            #   · 一条 `pip install opensquilla[recommended]` 即可修复；
            #   · 之前却记成 init_failed + ERROR —— 用户看到启动日志里 12 条 ERROR
            #     会以为系统坏了，而不是"有个可选增强没装"。
            # 上一轮只修了 LFS 那一种，留下这半修；这里补齐。
            if error_kind == ROUTER_WEIGHTS_NOT_FETCHED:
                event = "v4_phase3.weights_not_fetched"
            elif error_kind == ROUTER_PYTHON_DEPENDENCY_MISSING:
                event = "v4_phase3.deps_missing"
            else:
                event = "v4_phase3.init_failed"
            # 级别同样是契约：可一条命令修复的降级 → warning；真损坏 → error。
            emit = (
                log.warning
                if error_kind in REPAIRABLE_ROUTER_DEGRADATIONS
                else log.error
            )
            emit(
                event,
                bundle_dir=str(self.bundle_dir),
                error=str(exc),
                runtime_error_kind=error_kind,
                hint=router_runtime_hint(error_kind),
            )
            if require_router_runtime:
                raise RuntimeError(f"failed to initialize V4 Phase 3 router: {exc}") from exc

    def status(self) -> dict[str, Any]:
        """Read-only snapshot of this strategy's load outcome.

        Mirrors the gateway's ``router_runtime_status()`` shape but is scoped to
        one instance, so the doctor can quote the *specific* bundle dir and
        failure kind rather than the process-global strategy cache.
        """

        return {
            "available": self._available,
            "bundleDir": str(self.bundle_dir),
            "modelVersion": self._model_version,
            "featureSchemaVersion": self._feature_schema_version,
            "errorKind": self._init_error_kind,
            "error": self._init_error,
            "hint": (
                router_runtime_hint(self._init_error_kind) if self._init_error_kind else ""
            ),
        }

    @property
    def error(self) -> str | None:
        """Why this strategy is unavailable (``None`` when it loaded fine).

        Exposed under the same attribute name the heuristic/unavailable
        fallbacks already use, so ``router_runtime_status()`` and the gateway's
        deep validator can quote — and classify — the real cause instead of a
        generic "did not become available".
        """

        return self._init_error

    def _init_runtime(self, use_aux_head: bool | None) -> None:
        self._validate_bundle()
        self._config = (
            yaml.safe_load((self.bundle_dir / "router.runtime.yaml").read_text(encoding="utf-8"))
            or {}
        )
        self._model_version = self._read_model_version()
        self._feature_schema_version = self._compute_feature_schema_version()
        # Hand the capture toggles to the inference core via its config dict.
        self._config["emit_train_features"] = self._emit_train_features
        self._config["emit_raw_bge"] = self._emit_train_features and self._emit_raw_bge

        with runtime_src_import_path(self.bundle_dir):
            from src.router.inference.core import InferenceCore
            from src.router.inference.types import InferenceRequest

            resolved_aux_head = (
                bool(self._config.get("v4", {}).get("aux_head_inference", False))
                if use_aux_head is None
                else use_aux_head
            )
            self._request_type = InferenceRequest
            self._core = InferenceCore.from_model_dir(
                self.bundle_dir,
                self._config,
                use_aux_head=resolved_aux_head,
            )
        self._available = True

    def _validate_bundle(self) -> None:
        missing = [
            name for name in _REQUIRED_BUNDLE_FILES if not (self.bundle_dir / name).exists()
        ]
        if missing:
            raise FileNotFoundError(f"missing V4 bundle files: {missing}")
        self._validate_artifact_manifest()

    def _validate_artifact_manifest(self) -> None:
        problems = scan_manifest(self.bundle_dir).problems
        if problems:
            detail = "; ".join(problems[:5])
            if len(problems) > 5:
                detail += f"; ... {len(problems) - 5} more"
            raise RuntimeError(f"incomplete V4 router artifact bundle: {detail}")

    def _read_model_version(self) -> str:
        return read_bundle_version(self.bundle_dir)

    def _compute_feature_schema_version(self) -> str:
        """Hash the fitted feature projections so captured ``features_390`` are
        only ever mixed with samples produced under the same basis.

        Re-fitting TF-IDF/SVD/PCA changes these bytes, which invalidates older
        captured vectors (different coordinate system); the offline trainer keys
        on this to start a fresh dataset generation. Hashing artifact bytes is
        more robust than trusting a hand-bumped version string.
        """

        digest = hashlib.sha256()
        for rel in (
            "inference_manifest.json",
            "features/meta.json",
            "features/config.pkl",
            "features/tfidf.pkl",
            "features/svd.pkl",
            "features/bge_pca.joblib",
        ):
            path = self.bundle_dir / rel
            if path.exists():
                digest.update(path.read_bytes())
        return digest.hexdigest()[:16] if digest.digest() else "unknown"

    async def classify(
        self,
        message: str,
        valid_tiers: list[str],
        routing_history: list[dict] | None = None,
        prev_assistant_text: str | None = None,
        prev_assistant_usage: dict | None = None,
        history_user_texts: list[str] | None = None,
        flags_text_override: str | None = None,
    ) -> tuple[str, float, str, dict]:
        """Classify a turn into OpenSquilla tier format."""
        if not self._available or self._core is None or self._request_type is None:
            return self._unavailable_classify(valid_tiers)

        try:
            request = self._build_request(
                message,
                routing_history or [],
                prev_assistant_text=prev_assistant_text,
                prev_assistant_usage=prev_assistant_usage,
                history_user_texts=history_user_texts,
                flags_text_override=flags_text_override,
            )
            result = self._core.predict(request)
            return self._map_result(result, valid_tiers, message)
        except Exception as exc:
            log.warning("v4_phase3.predict_failed", error=str(exc), exc_info=True)
            if self._require_router_runtime:
                raise
            return self._unavailable_classify(valid_tiers)

    def _unavailable_classify(
        self,
        valid_tiers: list[str],
    ) -> tuple[str, float, str, dict]:
        if not self._degraded_warned:
            self._degraded_warned = True
            log.warning(
                "v4_phase3.degraded",
                detail=(
                    "v4_phase3 runtime unavailable; every turn falls back to the "
                    "default tier (source=v4_unavailable). Install "
                    "opensquilla[recommended] and verify the bundle to restore "
                    "ML routing, or set require_router_runtime=true to fail fast."
                ),
            )
        tier = _find_valid_tier(DEFAULT_TEXT_TIER, valid_tiers)
        route_class = next(
            (key for key, value in _ROUTE_CLASS_TO_TIER.items() if value == tier),
            "R1",
        )
        return (
            tier,
            0.0,
            "v4_unavailable",
            {
                "route_class": route_class,
                "top1_label": route_class,
                "thinking_mode": "T1",
                "prompt_policy": "P1",
                "model_version": self._model_version,
            },
        )

    def _build_request(
        self,
        message: str,
        routing_history: list[dict],
        *,
        prev_assistant_text: str | None = None,
        prev_assistant_usage: dict | None = None,
        history_user_texts: list[str] | None = None,
        flags_text_override: str | None = None,
    ) -> Any:
        if history_user_texts is None:
            history_texts = [str(entry["text"]) for entry in routing_history if entry.get("text")]
        else:
            history_texts = [str(text) for text in history_user_texts if text]
        context_tokens_est = max(
            0,
            (
                len(message)
                + sum(len(text) for text in history_texts)
                + len(prev_assistant_text or "")
            )
            // 4,
        )
        decisions: list[Any] = []
        for entry in routing_history:
            route_class = entry.get("final_route_class") or entry.get("route_class")
            if route_class:
                decisions.append(
                    SimpleNamespace(
                        route_class=str(route_class),
                        difficulty=float(
                            entry.get("difficulty_score", entry.get("difficulty", 0.0)) or 0.0
                        ),
                        margin=float(entry.get("margin", 0.0) or 0.0),
                    )
                )

        request_type = self._request_type
        if request_type is None:
            raise RuntimeError("V4 Phase 3 router request type is not initialized")

        return request_type(
            current_user_text=message,
            history_user_texts=history_texts,
            prev_assistant_text=prev_assistant_text,
            prev_assistant_usage=prev_assistant_usage,
            prev_route_decisions=decisions,
            flags_text_override=flags_text_override,
            context_metadata={
                "turn_index": len(routing_history),
                "history_user_turn_count": len(history_texts),
                "context_tokens_est": context_tokens_est,
                "has_code_block": "```" in message,
                "has_prev_assistant": bool(prev_assistant_text),
            },
        )

    def _map_result(
        self,
        result: Any,
        valid_tiers: list[str],
        message: str,
    ) -> tuple[str, float, str, dict]:
        decision = result.decision
        route_class = str(getattr(decision, "route_class", "R1"))
        tier = _ROUTE_CLASS_TO_TIER.get(route_class, DEFAULT_TEXT_TIER)
        if tier not in valid_tiers:
            tier = _find_valid_tier(tier, valid_tiers)

        probabilities = dict(getattr(result, "probabilities", {}) or {})
        confidence = float(probabilities.get(route_class, 0.0))
        thinking_mode = getattr(decision, "thinking_mode", None)
        prompt_policy = getattr(decision, "prompt_policy", None)
        if thinking_mode is None:
            log.warning("v4_phase3.missing_thinking_mode", route_class=route_class)
            thinking_mode = "T0"
        if prompt_policy is None:
            log.warning("v4_phase3.missing_prompt_policy", route_class=route_class)
            prompt_policy = "P0"

        difficulty = float(getattr(decision, "difficulty_score", 0.0))
        intermediates = dict(getattr(result, "intermediates", {}) or {})
        extra: dict[str, Any] = {
            "route_class": route_class,
            "top1_label": route_class,
            "probabilities": probabilities,
            "difficulty": difficulty,
            "difficulty_score": difficulty,
            "margin": float(getattr(decision, "margin", 0.0)),
            "thinking_mode": str(thinking_mode),
            "prompt_policy": str(prompt_policy),
            "flags": dict(getattr(decision, "flags", {}) or {}),
            "aux_decision_probs": getattr(result, "aux_decision_probs", None),
            "aux_downgrade_applied": bool(getattr(decision, "aux_downgrade_applied", False)),
            "sticky_applied": bool(getattr(decision, "sticky_applied", False)),
            "selected_model": getattr(decision, "selected_model", None),
            "model_version": self._model_version,
        }
        prompt_hint = self._prompt_hint(str(prompt_policy), message) or intermediates.get(
            "prompt_hint"
        )
        if prompt_hint:
            extra["prompt_hint"] = str(prompt_hint)
        # Self-learning: carry the captured feature vectors under a private key.
        # The router step pops this out of routing_extra before logging/history
        # so the large arrays never reach decision logs or routing history.
        features_390 = intermediates.get("features_390")
        if features_390 is not None:
            extra["_train_features"] = {
                "features_390": features_390,
                "raw_bge_1536": intermediates.get("raw_bge_1536"),
                "feature_schema_version": self._feature_schema_version,
            }
        return tier, confidence, self.source, extra

    def _prompt_hint(self, prompt_policy: str, message: str | None = None) -> str | None:
        policy_cfg = self._config.get("prompt_policies", {}).get(prompt_policy, {})
        return select_localized_prompt_hint(policy_cfg, message)
