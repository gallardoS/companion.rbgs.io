import { useRef, type KeyboardEvent } from 'react';

export const tabs = [
  { id: 'home', label: 'Home', title: 'Home' },
  { id: 'history', label: 'History', title: 'History' },
  { id: 'settings', label: 'Settings', title: 'Settings' },
] as const;

export type TabId = typeof tabs[number]['id'];

export function Icon({ name }: { name: TabId | 'captures' | 'overview' | 'account' | 'folder' | 'addon' | 'game' | 'history' | 'arrow' | 'check' | 'reload' | 'close' | 'copy' }) {
  const paths = {
    home: <><path d="m3 10 9-7 9 7v9a2 2 0 0 1-2 2h-4v-7H9v7H5a2 2 0 0 1-2-2z" /></>,
    overview: <><rect x="3" y="3" width="7" height="7" rx="1.5" /><rect x="14" y="3" width="7" height="7" rx="1.5" /><rect x="3" y="14" width="7" height="7" rx="1.5" /><rect x="14" y="14" width="7" height="7" rx="1.5" /></>,
    settings: <><path d="M4 7h16M4 17h16" /><circle cx="9" cy="7" r="3" fill="currentColor" stroke="none" /><circle cx="15" cy="17" r="3" fill="currentColor" stroke="none" /></>,
    account: <><circle cx="12" cy="8" r="3.5" /><path d="M5 21v-2a7 7 0 0 1 14 0v2" /></>,
    folder: <path d="M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z" />,
    addon: <><rect x="3" y="3" width="7" height="7" rx="1.5" /><rect x="3" y="14" width="7" height="7" rx="1.5" /><rect x="14" y="14" width="7" height="7" rx="1.5" /><path d="M14 6h7m-3.5-3.5v7" /></>,
    game: <><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 21h8m-4-5v5" /></>,
    history: <><path d="M3 11a9 9 0 1 1 2.6 7M3 4v7h7m2-4v5l3 2" /></>,
    captures: <><rect x="4" y="3" width="16" height="18" rx="2" /><path d="M8 8h8M8 12h8M8 16h5" /></>,
    arrow: <path d="M5 12h14m-5-5 5 5-5 5" />,
    check: <path d="m5 12 4 4L19 6" />,
    close: <path d="m6 6 12 12M18 6 6 18" />,
    copy: <><rect x="8" y="8" width="12" height="13" rx="2" /><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3" /></>,
    reload: <><path d="M19 5v4h-4M5 19v-4h4" /><path d="M5.2 9a7 7 0 0 1 11.75-2.05L19 9M5 15l2.05 2.05A7 7 0 0 0 18.8 15" /></>,
  };
  return <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name]}</svg>;
}

export function Navigation({ active, onChange }: { active: TabId; onChange: (tab: TabId) => void }) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  function navigate(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    let next = index;
    if (event.key === 'ArrowRight') next = (index + 1) % tabs.length;
    else if (event.key === 'ArrowLeft') next = (index + tabs.length - 1) % tabs.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = tabs.length - 1;
    else return;
    event.preventDefault();
    onChange(tabs[next].id);
    buttons.current[next]?.focus();
  }
  return <div className="navigation-wrap"><div className="companion-tabs" role="tablist" aria-label="Companion features">
    {tabs.map((tab, index) => <button key={tab.id} ref={element => { buttons.current[index] = element; }} type="button" role="tab"
      id={`tab-${tab.id}`} aria-controls={`panel-${tab.id}`} aria-selected={active === tab.id} tabIndex={active === tab.id ? 0 : -1}
      onClick={() => onChange(tab.id)} onKeyDown={event => navigate(event, index)}><Icon name={tab.id} />{tab.label}</button>)}
  </div></div>;
}
