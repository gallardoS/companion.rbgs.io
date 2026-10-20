import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Activity = { accountId: string | null; version: number; captureIds: string[]; openCaptureId: string | null; openSequence: number };
export function useAssignmentActivity(accountId: string, enabled: boolean) {
  const [snapshot, setSnapshot] = useState<Activity | null>(null);
  useEffect(() => {
    if (!enabled || !accountId) { setSnapshot(null); return; }
    let disposed = false;
    let inFlight = false;
    let timer: ReturnType<typeof setTimeout>;
    const read = async () => {
      if (disposed || inFlight) return;
      clearTimeout(timer); inFlight = true;
      try {
        const result = await invoke<Activity>('assignment_activity');
        if (!result || ![result.version, result.openSequence].every(n => Number.isSafeInteger(n) && n >= 0)
          || !Array.isArray(result.captureIds) || result.captureIds.length > 50 || !result.captureIds.every(id => typeof id === 'string' && /^[0-9a-f-]{36}$/i.test(id))
          || result.openCaptureId !== null && !result.captureIds.includes(result.openCaptureId)) throw Error();
        if (!disposed) setSnapshot(result.accountId === accountId ? result : null);
      } catch { if (!disposed) setSnapshot(null); }
      finally { inFlight = false; if (!disposed) timer = setTimeout(() => void read(), 2000); }
    };
    void read();
    const focus = () => void read();
    window.addEventListener('focus', focus);
    return () => { disposed = true; clearTimeout(timer); window.removeEventListener('focus', focus); };
  }, [accountId, enabled]);
  return enabled && snapshot?.accountId === accountId ? snapshot : null;
}
