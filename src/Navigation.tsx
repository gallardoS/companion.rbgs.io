import { useRef, type KeyboardEvent } from 'react';

export const tabs = [
  { id: 'account', label: 'Account', title: 'Your connection', description: 'Connect this PC to your rbgs.io account and manage its permissions.' },
  { id: 'game', label: 'Game data', title: 'Game data', description: 'Choose your Forever installation and the account the companion reads.' },
  { id: 'history', label: 'Match history', title: 'Match history', description: 'Explore completed captures and their final scoreboards, saved on this PC.' },
  { id: 'captures', label: 'Captures', title: 'Captures', description: 'Manage your test assignments and send saved captures to rbgs.io.' },
] as const;

export type TabId = typeof tabs[number]['id'];

export function Icon({ name }: { name: TabId | 'arrow' | 'check' | 'reload' }) {
  const paths = {
    account: <><circle cx="12" cy="8" r="3.5" /><path d="M5 21v-2a7 7 0 0 1 14 0v2" /></>,
    game: <><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 21h8m-4-5v5" /></>,
    history: <><path d="M3 11a9 9 0 1 1 2.6 7M3 4v7h7m2-4v5l3 2" /></>,
    captures: <><path d="M12 16V3m-5 5 5-5 5 5M4 15v4a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4" /></>,
    arrow: <path d="M5 12h14m-5-5 5 5-5 5" />,
    check: <path d="m5 12 4 4L19 6" />,
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
