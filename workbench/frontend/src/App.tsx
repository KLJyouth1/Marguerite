import { useWorkbenchStore } from './store/workbenchStore';
import { PANELS, panelRegistry } from './registry';
import { Icon } from './components/ui/icons';

function ToastStack(): JSX.Element {
  const toasts = useWorkbenchStore((s) => s.toasts);
  const dismiss = useWorkbenchStore((s) => s.dismissToast);
  return (
    <div className="toaststack" aria-live="polite" aria-label="Notifications">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast--${t.kind}`} role="status">
          <Icon name={t.kind === 'error' ? 'alert' : t.kind === 'success' ? 'check' : 'spark'} size={15} />
          <span className="toast__msg">{t.message}</span>
          <button type="button" className="toast__close" aria-label="Dismiss notification" onClick={() => dismiss(t.id)}>
            <Icon name="close" size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}

export default function App(): JSX.Element {
  const activePanel = useWorkbenchStore((s) => s.activePanel);
  const setActivePanel = useWorkbenchStore((s) => s.setActivePanel);
  const theme = useWorkbenchStore((s) => s.theme);
  const toggleTheme = useWorkbenchStore((s) => s.toggleTheme);

  const ActivePanel = panelRegistry[activePanel];
  const meta = PANELS.find((p) => p.id === activePanel) ?? PANELS[0];

  return (
    <div className="app">
      <aside className="rail" aria-label="Brand">
        <span className="rail__logo" aria-hidden="true">
          <Icon name="logo" size={22} />
        </span>
        <span className="rail__brand">MQ</span>
        <button
          type="button"
          className="rail__theme"
          onClick={toggleTheme}
          aria-label={theme === 'light' ? 'Switch to dark theme' : 'Switch to light theme'}
          title="Toggle theme"
        >
          <Icon name={theme === 'light' ? 'moon' : 'sun'} size={18} />
        </button>
      </aside>

      <nav className="nav" aria-label="Workbench sections">
        <div className="nav__brand">
          <h1 className="nav__title">Marguerite</h1>
          <p className="nav__subtitle">File Intelligence Workbench</p>
        </div>
        <ul className="nav__list">
          {PANELS.map((panel) => (
            <li key={panel.id}>
              <button
                type="button"
                className={`nav__item ${activePanel === panel.id ? 'nav__item--active' : ''}`}
                aria-current={activePanel === panel.id ? 'page' : undefined}
                onClick={() => setActivePanel(panel.id)}
              >
                <span className="nav__icon" aria-hidden="true">
                  <Icon name={panel.icon} size={18} />
                </span>
                <span className="nav__text">
                  <span className="nav__label">{panel.label}</span>
                  <span className="nav__desc">{panel.description}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      </nav>

      <main className="main">
        <header className="main__head">
          <span className="main__icon" aria-hidden="true">
            <Icon name={meta.icon} size={20} />
          </span>
          <div>
            <h2 className="main__title">{meta.label}</h2>
            <p className="main__desc">{meta.description}</p>
          </div>
        </header>
        <div className="main__body">
          <ActivePanel />
        </div>
      </main>

      <ToastStack />
    </div>
  );
}
