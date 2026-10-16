import { useCallback, useEffect } from 'react';
import type {
  CapabilitiesInfo,
  HarnessAdapter,
  HealthStatus,
  MemoryLayerInfo,
} from '../types/contract';
import type { ExpertRole } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Icon, type IconName } from '../components/ui/icons';
import { Button } from '../components/ui/Button';
import { ErrorState, LoadingState } from '../components/ui/States';
import { PANELS, type PanelId } from '../registry';

interface OverviewSnapshot {
  health: HealthStatus;
  capabilities: CapabilitiesInfo;
  layers: MemoryLayerInfo[];
  experts: ExpertRole[];
  adapters: HarnessAdapter[];
}

/** Ordered quick-nav entries — derived from the registry, no hardcoded ids. */
const QUICK_LINKS: ReadonlyArray<{ id: PanelId; icon: IconName }> = PANELS
  .filter((p) => p.id !== 'overview')
  .map((p) => ({ id: p.id, icon: p.icon }));

function formatCount(n: number): string {
  return n.toLocaleString();
}

export function OverviewPanel(): JSX.Element {
  const snapshot = useAsync<OverviewSnapshot>();
  const setActivePanel = useWorkbenchStore((s) => s.setActivePanel);
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  const load = useCallback(async () => {
    await snapshot.run(async () => {
      const [health, capabilities, layers, experts, adapters] = await Promise.all([
        api.getHealth(),
        api.getCapabilities(),
        api.getMemoryLayers(),
        api.getExperts(),
        api.getHarnessAdapters(),
      ]);
      return { health, capabilities, layers, experts, adapters };
    });
  }, [snapshot]);

  useEffect(() => {
    load().catch(() => {
      // surfaced through the async error state; nothing to rethrow in effects
    });
    // Mount-only telemetry fetch: the dashboard exposes an explicit Refresh
    // control for manual reloads.
  }, [load]);

  if (snapshot.loading && !snapshot.data) return <LoadingState label="Reading workbench telemetry…" />;
  if (snapshot.error && !snapshot.data) {
    return (
      <ErrorState
        title="Workbench backend unreachable"
        message={snapshot.error}
        onRetry={() => {
          load().catch(() => undefined);
        }}
      />
    );
  }
  if (!snapshot.data) return <LoadingState label="Preparing overview…" />;

  const { health, capabilities, layers, experts, adapters } = snapshot.data;
  const backend = capabilities.memory_backend;
  const nativeCaps = capabilities.native_capabilities;
  const totalMemories = layers.reduce((sum, l) => sum + l.count, 0);
  const totalCapacity = layers.reduce((sum, l) => sum + l.capacity, 0);
  const availableAdapters = adapters.filter((a) => a.available);

  const retryWithToast = (): void => {
    load().then(
      () => pushToast('success', 'Telemetry refreshed'),
      (err: unknown) => pushToast('error', err instanceof Error ? err.message : 'Refresh failed'),
    );
  };

  return (
    <div className="feature">
      <div className="ov-toolbar">
        <span className={`status-dot ${health.status === 'ok' ? 'status-dot--on' : 'status-dot--warn'}`}>
          {health.status}
        </span>
        <span className="chip chip--mono">service {capabilities.service}</span>
        <span className="chip chip--mono">bridge v{nativeCaps.bridge_version}</span>
        <Button variant="ghost" size="sm" icon="spark" onClick={retryWithToast} disabled={snapshot.loading}>
          Refresh
        </Button>
      </div>

      <section className="ov-stats" aria-label="Key metrics">
        <article className="stat">
          <header className="stat__head"><Icon name="layers" size={15} /> Memory</header>
          <p className="stat__value">{formatCount(totalMemories)}</p>
          <p className="stat__hint">of {formatCount(totalCapacity)} capacity · {backend.active} store</p>
        </article>
        <article className="stat">
          <header className="stat__head"><Icon name="brain" size={15} /> Experts</header>
          <p className="stat__value">{experts.length}</p>
          <p className="stat__hint">distilled roles ready</p>
        </article>
        <article className="stat">
          <header className="stat__head"><Icon name="cpu" size={15} /> Adapters</header>
          <p className="stat__value">{availableAdapters.length}/{adapters.length}</p>
          <p className="stat__hint">
            {availableAdapters.length > 0 ? availableAdapters.map((a) => a.id).join(', ') : 'internal loop only'}
          </p>
        </article>
        <article className="stat">
          <header className="stat__head"><Icon name="pulse" size={15} /> Backend</header>
          <p className="stat__value stat__value--sm">{backend.backend}</p>
          <p className="stat__hint" title={backend.db_path ?? undefined}>
            {backend.is_core_store ? 'native core store bound' : 'local fallback store'}
          </p>
        </article>
      </section>

      <section className="panel" aria-label="Memory layer occupancy">
        <header className="panel__head">
          <div className="panel__heading">
            <span className="panel__icon"><Icon name="layers" size={17} /></span>
            <div>
              <h3 className="panel__title">7-layer memory occupancy</h3>
              <p className="panel__subtitle">Live counts from the bridge-backed engine</p>
            </div>
          </div>
        </header>
        <div className="panel__body ov-layers">
          {layers.map((l) => (
            <div key={l.layer} className="ov-layer">
              <div className="ov-layer__row">
                <span className="ov-layer__id">{l.layer}</span>
                <span className="ov-layer__name">{l.name}</span>
                <span className="ov-layer__count">{formatCount(l.count)}<em>/{formatCount(l.capacity)}</em></span>
              </div>
              <div
                className="layercard__bar"
                role="meter"
                aria-valuemin={0}
                aria-valuemax={l.capacity}
                aria-valuenow={l.count}
                aria-label={`${l.layer} ${l.name} occupancy`}
              >
                <span
                  className="layercard__bar-fill"
                  style={{ width: `${Math.min(100, l.capacity > 0 ? (l.count / l.capacity) * 100 : 0)}%` }}
                />
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="ov-cols" aria-label="Capabilities and navigation">
        <div className="panel">
          <header className="panel__head">
            <div className="panel__heading">
              <span className="panel__icon"><Icon name="check" size={16} /></span>
              <div><h3 className="panel__title">Native capability manifest</h3></div>
            </div>
          </header>
          <div className="panel__body">
            <ul className="ov-caplist">
              {([
                ['memory_7layer', '7-layer memory'],
                ['file_intel', 'File intelligence'],
                ['knowledge_base', 'Hybrid knowledge base'],
                ['distilled_experts', 'Distilled experts'],
                ['research', 'Online research'],
              ] as const).map(([key, label]) => (
                <li key={key} className="ov-cap">
                  <span className={`status-dot ${nativeCaps[key] ? 'status-dot--on' : 'status-dot--off'}`} />
                  {label}
                </li>
              ))}
            </ul>
          </div>
        </div>

        <div className="panel">
          <header className="panel__head">
            <div className="panel__heading">
              <span className="panel__icon"><Icon name="chevronRight" size={16} /></span>
              <div><h3 className="panel__title">Jump to a capability</h3></div>
            </div>
          </header>
          <div className="panel__body ov-links">
            {QUICK_LINKS.map((link) => (
              <Button key={link.id} variant="secondary" size="sm" icon={link.icon} onClick={() => setActivePanel(link.id)}>
                {PANELS.find((p) => p.id === link.id)?.label ?? link.id}
              </Button>
            ))}
          </div>
        </div>
      </section>
    </div>
  );
}
