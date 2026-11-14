import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

type Progress = {
  version: string;
  downloaded: number;
  total: number | null;
  installing: boolean;
  failed: boolean;
};

function afterPaint(callback: () => void) {
  let frame = requestAnimationFrame(() => { frame = requestAnimationFrame(callback); });
  return () => cancelAnimationFrame(frame);
}

export function useCompanionUpdates() {
  const [available, setAvailable] = useState<string | null>(null);
  const [action, setAction] = useState<'check' | 'install' | null>(null);
  const [status, setStatus] = useState('');
  const [installError, setInstallError] = useState(false);
  const busy = useRef(false);

  useEffect(() => {
    let disposed = false;
    let receivedUpdate = false;
    let unlisten: (() => void) | undefined;
    const cancelPaint = afterPaint(() => { void invoke('companion_window_ready').catch(() => {}); });
    void listen<string | null>('companion-update-available', ({ payload }) => {
      if (disposed) return;
      receivedUpdate = true;
      setAvailable(payload);
      setStatus('');
      setInstallError(false);
    }).then(async (unsubscribe) => {
      if (disposed) return unsubscribe();
      unlisten = unsubscribe;
      const version = await invoke<string | null>('available_update');
      if (!disposed && !receivedUpdate) setAvailable(version);
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
      cancelPaint();
    };
  }, []);

  async function run(next: 'check' | 'install') {
    if (busy.current) return;
    busy.current = true;
    setAction(next);
    setStatus('');
    setInstallError(false);
    try {
      if (next === 'check') {
        const version = await invoke<string | null>('check_for_updates');
        setAvailable(version);
        if (!version) setStatus('The companion is up to date.');
      } else if (!await invoke<boolean>('install_update')) {
        setAvailable(null);
        setStatus('The companion is up to date.');
      }
    } catch (error) {
      setStatus(typeof error === 'string' ? error : 'Could not complete the update. Try again.');
      if (next === 'install') setInstallError(true);
    } finally {
      busy.current = false;
      setAction(null);
    }
  }

  return { available, action, status, installError, check: () => void run('check'), install: () => void run('install') };
}

export function UpdateProgressWindow() {
  const [progress, setProgress] = useState<Progress | null>(null);
  const hasProgress = progress !== null;

  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;
    async function poll() {
      try {
        const next = await invoke<Progress | null>('update_window_progress');
        if (!stopped && next) setProgress(next);
      } finally {
        if (!stopped) timer = window.setTimeout(() => void poll().catch(() => {}), 150);
      }
    }
    void poll().catch(() => {});
    return () => { stopped = true; window.clearTimeout(timer); };
  }, []);

  useEffect(() => {
    if (hasProgress) return afterPaint(() => { void invoke('update_window_ready').catch(() => {}); });
  }, [hasProgress]);

  const percent = progress?.total ? Math.min(100, Math.floor(progress.downloaded * 100 / progress.total)) : undefined;
  let message = 'Preparing update…';
  if (progress?.failed) {
    message = progress.installing
      ? 'The companion did not reopen. Try opening it from the Start menu.'
      : 'The update could not finish downloading. You can keep using your current version.';
  } else if (progress?.installing) {
    message = 'Installing the update… The companion will reopen automatically.';
  } else if (progress) {
    message = `Downloading the new version…${percent === undefined ? '' : ` ${percent}%`}`;
  }

  return <main className="update-window" aria-labelledby="update-title">
    <h2 id="update-title">Updating companion{progress ? ` to v${progress.version}` : ''}</h2>
    <p role="status" aria-live="polite">{message}</p>
    {progress?.failed
      ? <button autoFocus onClick={() => void invoke('close_update_window').catch(() => {})}>Close</button>
      : <progress max={100} value={progress?.installing ? undefined : percent} aria-label="Update progress" />}
  </main>;
}
