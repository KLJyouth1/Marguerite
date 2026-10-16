from __future__ import annotations

from opensquilla.router_runtime_diagnostics import (
    MACOS_LIBOMP_MISSING,
    ROUTER_ASSETS_MISSING,
    ROUTER_NATIVE_DEPENDENCY_MISSING,
    ROUTER_PYTHON_DEPENDENCY_MISSING,
    ROUTER_WEIGHTS_NOT_FETCHED,
    WINDOWS_VC_RUNTIME_MISSING,
    classify_router_runtime_error,
    router_runtime_hint,
    router_runtime_operator_message,
)


def test_router_runtime_classifier_detects_macos_libomp_failure() -> None:
    error = RuntimeError(
        "dlopen(.../lightgbm/lib/lib_lightgbm.dylib, 0x0006): Library not loaded: "
        "@rpath/libomp.dylib Referenced from: .../lib_lightgbm.dylib Reason: tried"
    )

    assert classify_router_runtime_error(error) == MACOS_LIBOMP_MISSING
    assert "brew install libomp" in router_runtime_operator_message(error)


def test_router_runtime_classifier_detects_windows_vc_failure() -> None:
    error = RuntimeError(
        "DLL load failed while importing onnxruntime_pybind11_state: "
        "The specified module could not be found."
    )

    assert classify_router_runtime_error(error) == WINDOWS_VC_RUNTIME_MISSING
    assert "vc_redist.x64.exe" in router_runtime_operator_message(error)


def test_router_runtime_classifier_detects_assets_and_generic_native_failures() -> None:
    assert (
        classify_router_runtime_error("missing V4 bundle files in /tmp/router: ['runtime_src']")
        == ROUTER_ASSETS_MISSING
    )
    assert (
        classify_router_runtime_error("lib_lightgbm.so: cannot open shared object file")
        == ROUTER_NATIVE_DEPENDENCY_MISSING
    )


def test_router_runtime_classifier_detects_wrapped_python_dependency_failure() -> None:
    error = RuntimeError("failed to initialize V4 Phase 3 router")
    error.__cause__ = ImportError("No module named 'lightgbm'")

    assert classify_router_runtime_error(error) == ROUTER_PYTHON_DEPENDENCY_MISSING


def test_router_runtime_classifier_separates_lfs_pointers_from_missing_assets() -> None:
    """A pointer bundle is fixable with one command; a missing one is not.

    Collapsing both into ``router_runtime_unavailable`` (the old behaviour for
    the pointer case, because size mismatch won the match) told operators to
    reinstall when ``git lfs pull`` was the actual fix.
    """

    pointer_error = (
        "incomplete V4 router artifact bundle: lgbm_main.bin: Git LFS pointer, not model data"
    )
    assert classify_router_runtime_error(pointer_error) == ROUTER_WEIGHTS_NOT_FETCHED

    # The pre-fix wording (size drift) must also route to the actionable kind,
    # since a pointer is always smaller than the artifact it replaces.
    legacy_wording = (
        "incomplete V4 router artifact bundle: lgbm_main.bin: size 133 != manifest 39684725"
    )
    assert classify_router_runtime_error(legacy_wording) == ROUTER_WEIGHTS_NOT_FETCHED

    # A genuinely missing bundle is still its own kind.
    assert (
        classify_router_runtime_error("missing V4 bundle files: ['runtime_src']")
        == ROUTER_ASSETS_MISSING
    )

    hint = router_runtime_hint(ROUTER_WEIGHTS_NOT_FETCHED)
    assert "git lfs pull" in hint
    assert "reinstall" in hint.lower()
    assert "router_weights_not_fetched" not in hint
