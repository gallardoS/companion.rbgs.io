import { createContext, useContext, useState, type ReactNode } from 'react';

type PrivacyPreferences = { battleNet: boolean; wowPath: boolean; wowAccount: boolean };
type PreferenceError = 'read' | 'write' | null;
const key = 'rbgs.privacy.v1';
const dots = '••••••••';
const defaults: PrivacyPreferences = { battleNet: false, wowPath: false, wowAccount: false };
const hidden: PrivacyPreferences = { battleNet: true, wowPath: true, wowAccount: true };

function readPreferences(): { preferences: PrivacyPreferences; error: PreferenceError } {
  try {
    const stored = localStorage.getItem(key);
    if (stored === null) return { preferences: defaults, error: null };
    const value = JSON.parse(stored);
    if (!value || !['battleNet', 'wowPath', 'wowAccount'].every(name => typeof value[name] === 'boolean')) throw Error('Invalid privacy preferences');
    return { preferences: { battleNet: value.battleNet, wowPath: value.wowPath, wowAccount: value.wowAccount }, error: null };
  } catch { return { preferences: hidden, error: 'read' }; }
}

function maskPath(value: string): string {
  const parts = value.split(/[\\/]+/).filter(Boolean);
  const wow = parts.findIndex(part => part.toLowerCase() === 'world of warcraft');
  const visible = wow >= 0 ? parts.slice(wow) : parts.slice(-1);
  return `${dots}\\${visible.join('\\')}`;
}

type PrivacyContextValue = {
  preferences: PrivacyPreferences;
  error: PreferenceError;
  change: (name: keyof PrivacyPreferences, enabled: boolean) => void;
  accountName: (value: string) => string;
  wowAccount: (value: string) => string;
  wowPath: (value: string) => string;
};

const PrivacyContext = createContext<PrivacyContextValue | null>(null);

export function PrivacyProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState(readPreferences);
  function change(name: keyof PrivacyPreferences, enabled: boolean) {
    const preferences = { ...state.preferences, [name]: enabled };
    let error: PreferenceError = null;
    try { localStorage.setItem(key, JSON.stringify(preferences)); }
    catch { error = 'write'; }
    setState({ preferences, error });
  }
  const { preferences, error } = state;
  return <PrivacyContext.Provider value={{ preferences, error, change,
    accountName: value => preferences.battleNet ? value.replace(/#\d+$/, `#${dots}`) : value,
    wowAccount: value => preferences.wowAccount ? `${dots}${value.match(/#\d+$/)?.[0] ?? ''}` : value,
    wowPath: value => {
      const path = value.replace(/^\\\\\?\\/, '');
      return preferences.wowPath ? maskPath(path) : path;
    },
  }}>{children}</PrivacyContext.Provider>;
}

export function usePrivacy() {
  const value = useContext(PrivacyContext);
  if (!value) throw Error('PrivacyProvider is required');
  return value;
}

export function PrivacySettings() {
  const { preferences, change, error } = usePrivacy();
  const options = [
    { name: 'battleNet', title: 'Hide Battle.net numbers', description: 'Keep the name visible and hide the numbers after #.' },
    { name: 'wowPath', title: 'Hide WoW folder location', description: 'Hide the path before World of Warcraft. Keep the installation folders visible.' },
    { name: 'wowAccount', title: 'Hide WoW account', description: 'Hide the account identifier and keep its suffix, such as #1.' },
  ] as const;
  return <section className="settings-section" aria-labelledby="settings-privacy">
    <div className="section-heading"><h2 id="settings-privacy" tabIndex={-1}>Privacy</h2></div>
    <div className="section-body"><div className="privacy-options">{options.map(option => <label className="privacy-option" key={option.name}>
      <span><strong>{option.title}</strong><span>{option.description}</span></span>
      <input type="checkbox" checked={preferences[option.name]} aria-label={option.title} onChange={event => change(option.name, event.target.checked)} />
    </label>)}</div>
    {error && <p className="status-message" role="alert">{error === 'read' ? 'Could not read privacy preferences. Personal details are hidden until you change these options.' : 'Could not save privacy preferences. Your choices apply until you close the companion.'}</p>}
    </div>
  </section>;
}
