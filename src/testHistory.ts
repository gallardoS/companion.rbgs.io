import { useState } from 'react';

export type ClosedTest = { captureId: string; character: { name: string; realm: string | null }; armUntil: string; cancelled: boolean };

const hiddenTestsKey = (accountId: string) => `rbgs.hidden-tests.v1.${accountId}`;
function readHiddenTests(accountId: string): Set<string> {
  try {
    const stored = localStorage.getItem(hiddenTestsKey(accountId));
    if (!stored || stored.length > 8192) return new Set();
    const ids: unknown = JSON.parse(stored);
    if (Array.isArray(ids) && ids.length <= 200 && ids.every(id => typeof id === 'string' && /^[0-9a-f-]{36}$/i.test(id))) return new Set(ids);
  } catch {}
  return new Set();
}

export function useHiddenTests(accountId: string) {
  const [hidden, setHidden] = useState(() => readHiddenTests(accountId));
  const [error, setError] = useState(false);
  const [lastHidden, setLastHidden] = useState<string | null>(null);
  function save(ids: Set<string>) {
    try {
      const bounded = new Set([...ids].slice(-200));
      localStorage.setItem(hiddenTestsKey(accountId), JSON.stringify([...bounded]));
      setHidden(bounded); setError(false);
      return true;
    } catch { setError(true); return false; }
  }
  return {
    hidden, error, lastHidden,
    hide: (id: string) => { if (save(new Set([...hidden, id]))) setLastHidden(id); },
    undo: () => { if (lastHidden && save(new Set([...hidden].filter(id => id !== lastHidden)))) setLastHidden(null); },
    showAll: () => { if (save(new Set())) setLastHidden(null); },
  };
}
