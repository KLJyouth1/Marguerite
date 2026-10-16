# Marks `tests/` as a package so cross-suite imports resolve.
#
# Several tests import helpers from sibling test modules, e.g.
#   from tests.unit.cli.tui.test_opentui_surface import FakeOpenTuiBridge
#   from tests.test_engine.test_runtime_meta_invoke_surfacing import _make_loader_with_meta
# Without this file pytest's default `prepend` import mode puts each test file's own
# directory on `sys.path` instead, so `tests.*` is not an importable package and those
# tests fail at collection with `ModuleNotFoundError: No module named 'tests.<x>'`.
#
# Do not delete this because it "looks unnecessary": it is load-bearing. Measured on
# this repository, adding it changed collection from
#   20539 tests collected, 22 errors   ->   20588 tests collected, 21 errors
# and turned `test_opentui_prefs.py::test_surface_persists_picker_confirmation`
# from a hard failure into a pass. It also switches pytest to package import mode,
# which makes test module names fully qualified (`tests.unit....`) and therefore
# collision-free across the ~1280 test files here.
