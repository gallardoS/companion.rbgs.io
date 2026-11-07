import { useEffect, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';

type Progress = { version: string; downloaded: number; total: number | null; installing: boolean };

export function AutoUpdate() {
  const started = useRef(false);
  const dialog = useRef<HTMLDialogElement>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const channel = new Channel<Progress>();
    channel.onmessage = setProgress;
    void invoke('update_on_startup', { progress: channel }).catch(() => {
      setError('The companion could not update. You can keep using this version and try again next time you open it.');
    });
  }, []);

  useEffect(() => {
    if (progress && !dialog.current?.open) dialog.current?.showModal();
  }, [progress]);

  if (!progress) return null;
  const percent = progress.total ? Math.min(100, Math.floor(progress.downloaded * 100 / progress.total)) : null;
  return <dialog ref={dialog} className="update-dialog" aria-labelledby="update-title" onCancel={event => { if (!error) event.preventDefault(); }}>
    <h2 id="update-title">Updating companion to v{progress.version}</h2>
    <div role="status" aria-live="polite">
      <p>{error || (progress.installing ? 'Installing the update… The companion will reopen automatically.' : `Downloading the new version${percent === null ? '…' : `… ${percent}%`}`)}</p>
    </div>
    {!error && <progress max={100} value={progress.installing ? undefined : percent ?? undefined} aria-label="Update download" />}
    {error && <button autoFocus onClick={() => { dialog.current?.close(); setProgress(null); }}>Continue</button>}
  </dialog>;
}
