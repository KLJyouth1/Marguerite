"""Vue route contract for the retired standalone Health destination."""

from spa_source import spa

SHARED_ROUTES = spa("src/router/sharedRoutes.ts")
HEALTH_VIEW = spa("src/views/HealthView.vue")


def test_health_deep_link_redirects_to_vue_overview() -> None:
    routes = SHARED_ROUTES.read_text(encoding="utf-8")

    assert "path: '/health'" in routes
    assert "redirect: '/overview'" in routes
    # Assert on the import path, not the bare view name: `EngineHealthView.vue`
    # (the live engine dashboard at /engine) contains "HealthView.vue" as a
    # substring, so a name-level check reports a false regression.
    assert "/views/HealthView.vue" not in routes
    assert not HEALTH_VIEW.exists()
