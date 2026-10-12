import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Summary = {
  schema: number; recordCount: number; clientVersion: string | null; clientBuild: string | null;
  clientInterface: string | null; observedCharacter: string | null; observedRealm: string | null;
  observedMap: string | null; lastRecordAt: number | null; latestScoreboardRows: number | null;
  rawWinner: string | null; lastCapturePhase: string | null; compatibility: string;
};
type SourceView = {
  installation: string | null; sources: { id: string; label: string }[]; selectedSource: string | null;
  status: string; message: string; data: Summary | null; fileModifiedAt: number | null; readAt: number | null;
};
const statuses = ['NOT_CONFIGURED', 'WRONG_CLIENT', 'MISSING_ADDON', 'UNSUPPORTED_ADDON', 'NO_SAVED_DATA', 'SELECT_SOURCE',
  'SOURCE_UNAVAILABLE', 'UNSTABLE_FILE', 'TOO_LARGE', 'INVALID_DATA', 'UNSUPPORTED_SCHEMA', 'UNSUPPORTED_CLIENT', 'UNKNOWN_CLIENT', 'READY'];
function nullableString(value: unknown): value is string | null { return value === null || typeof value === 'string'; }
function nullableNumber(value: unknown): value is number | null { return value === null || (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0); }
function validView(value: unknown): value is SourceView {
  if (!value || typeof value !== 'object') return false;
  const view = value as SourceView;
  if (!nullableString(view.installation) || !nullableString(view.selectedSource) || !statuses.includes(view.status)
    || typeof view.message !== 'string' || !nullableNumber(view.fileModifiedAt) || !nullableNumber(view.readAt)
    || !Array.isArray(view.sources) || view.sources.length > 64 || !view.sources.every(s => s && typeof s.id === 'string' && typeof s.label === 'string')) return false;
  if (view.data === null) return true;
  const data = view.data;
  return !!data && data.schema === 1 && Number.isSafeInteger(data.recordCount) && data.recordCount >= 0 && data.recordCount <= 300
    && [data.clientVersion, data.clientBuild, data.clientInterface, data.observedCharacter, data.observedRealm, data.observedMap, data.rawWinner, data.lastCapturePhase].every(nullableString)
    && [data.lastRecordAt, data.latestScoreboardRows].every(nullableNumber)
    && ['OBSERVED_DIAGNOSTIC_BUILD', 'UNSUPPORTED_CLIENT', 'UNKNOWN_CLIENT'].includes(data.compatibility);
}
function time(value: number | null): string {
  if (value === null) return 'Unknown';
  const date = new Date(value * 1000);
  return Number.isNaN(date.valueOf()) ? 'Unknown' : date.toLocaleString();
}

export function SavedVariables() {
  const id = useId();
  const [view, setView] = useState<SourceView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const generation = useRef(0);
  const resync = useRef(false);
  const [watchError, setWatchError] = useState<string | null>(null);
  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    inFlight.current = true;
    setBusy(true);
    invoke<unknown>('saved_variables_status').then(result => {
      if (disposed) return;
      if (!validView(result)) throw new Error('Invalid saved-data response');
      setView(result);
    }).catch(err => { if (!disposed) { setView(null); setError(String(err)); } })
      .finally(() => { if (!disposed) { inFlight.current = false; setBusy(false); } });
    return () => { disposed = true; mounted.current = false; };
  }, []);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      if (disposed) return;
      if (!inFlight.current) {
        const requestGeneration = generation.current;
        const refresh = resync.current;
        try {
          const result = await invoke<unknown>(refresh ? 'saved_variables_status' : 'saved_variables_update');
          if (!disposed && requestGeneration === generation.current && !inFlight.current) {
            if (result !== null) {
              if (!validView(result)) throw new Error('Invalid automatic saved-data response');
              setView(result); setError(null);
            }
            setWatchError(null);
            resync.current = false;
          }
        } catch (err) {
          if (!disposed && requestGeneration === generation.current && !inFlight.current) {
            setView(current => current ? { ...current, data: null, fileModifiedAt: null, readAt: null } : null);
            setWatchError(String(err));
            resync.current = true;
          }
        }
      }
      if (!disposed) timer = setTimeout(() => void poll(), 1000);
    }
    timer = setTimeout(() => void poll(), 1000);
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  async function run(command: string, args?: Record<string, string>) {
    if (inFlight.current) return;
    generation.current += 1; resync.current = true;
    inFlight.current = true; setBusy(true); setError(null);
    try {
      const result = await invoke<unknown>(command, args);
      if (!mounted.current) return;
      if (result === null && ['choose_forever_installation', 'scan_forever_installations'].includes(command)) return; // Native dialog cancelled.
      if (!validView(result)) throw new Error('Invalid saved-data response');
      setView(result);
    } catch (err) { if (mounted.current) { setView(current => current ? { ...current, data: null, fileModifiedAt: null, readAt: null } : null); setError(String(err)); } }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  }
  const data = view?.data;
  const warning = view && ['WRONG_CLIENT', 'MISSING_ADDON', 'UNSUPPORTED_ADDON', 'SOURCE_UNAVAILABLE', 'UNSTABLE_FILE', 'TOO_LARGE', 'INVALID_DATA', 'UNSUPPORTED_SCHEMA', 'UNSUPPORTED_CLIENT'].includes(view.status);
  return <section className="saved-variables" aria-labelledby={`${id}-heading`}>
    <h2 id={`${id}-heading`}>Saved game data</h2>
    <p>We check the usual World of Warcraft folders automatically. If Forever is installed elsewhere, scan a drive or games folder, or choose the installation directly.</p>
    <button disabled={busy} onClick={() => void run('choose_forever_installation')}>{view?.installation ? 'Change Forever folder' : 'Choose Forever folder'}</button>
    <button disabled={busy} onClick={() => void run('scan_forever_installations')}>Scan Folders</button>
    {view?.installation && <p className="local-path">{view.installation.replace(/^\\\\\?\\/, '')}</p>}
    {!!view?.sources.length && <div className="source-picker"><label htmlFor={`${id}-source`}>SavedVariables account</label>
      <select id={`${id}-source`} value={view.selectedSource ?? ''} disabled={busy}
        onChange={event => { if (event.target.value) void run('select_saved_variables_source', { sourceId: event.target.value }); }}>
        <option value="" disabled>Choose an account source</option>
        {view.sources.map(source => <option key={source.id} value={source.id}>{source.label}</option>)}
      </select>
    </div>}
    <button disabled={busy} onClick={() => void run('saved_variables_status')}>Refresh saved data</button>
    {busy && <p role="status">Checking local folders and saved data…</p>}
    {error && <p role="alert">{error}</p>}
    {watchError && <p role="alert">Automatic updates unavailable: {watchError}. Retrying…</p>}
    {!watchError && view?.selectedSource && <p className="saved-data-note">Watching selected saved file. Updates appear automatically after /reload or logout.</p>}
    {!busy && !error && !watchError && <p role={warning ? 'alert' : 'status'}>{view?.message ?? 'Loading saved-data settings…'}</p>}
    {!busy && !error && !watchError && data && <dl className="saved-data-summary">
      <div><dt>Saved file modified</dt><dd>{time(view.fileModifiedAt)}</dd></div>
      <div><dt>Read at</dt><dd>{time(view.readAt)}</dd></div>
      <div><dt>Client version / build / interface</dt><dd>{data.clientVersion ?? 'Unknown'} / {data.clientBuild ?? 'Unknown'} / {data.clientInterface ?? 'Unknown'}</dd></div>
      <div><dt>Retained diagnostic records</dt><dd>{data.recordCount}</dd></div>
      <div><dt>Observed character</dt><dd>{data.observedCharacter ?? 'Unknown'} · {data.observedRealm ?? 'Unknown'}</dd></div>
      <div><dt>Observed map</dt><dd>{data.observedMap ?? 'Unknown'}</dd></div>
      <div><dt>Last saved record timestamp</dt><dd>{time(data.lastRecordAt)}</dd></div>
      <div><dt>Latest scoreboard rows</dt><dd>{data.latestScoreboardRows ?? 'Unknown'}</dd></div>
      <div><dt>Raw winner code (unverified)</dt><dd>{data.rawWinner ?? 'Unknown'}</dd></div>
      <div><dt>Latest capture marker</dt><dd>{data.lastCapturePhase ?? 'Unknown'}</dd></div>
    </dl>}
    <p className="saved-data-note">Use /reload or log out in WoW to save new data. Keep the companion open for automatic updates; Refresh saved data is also available. SavedVariables do not update continuously. File freshness and device connectivity do not establish current in-game participation, account ownership or a verified match.</p>
  </section>;
}
