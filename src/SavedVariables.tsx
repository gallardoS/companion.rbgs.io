import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './Navigation';
import { errorDetail, Feedback } from './Feedback';
import { gameSetup } from './gameSetup';
import { usePrivacy } from './Privacy';

type Summary = {
  schema: number; recordCount: number; clientVersion: string | null; clientBuild: string | null;
  clientInterface: string | null; observedCharacter: string | null; observedRealm: string | null;
  observedMap: string | null; lastRecordAt: number | null; latestScoreboardRows: number | null;
  rawWinner: string | null; lastCapturePhase: string | null; compatibility: string;
};
type GameView = { status: string; message: string; fileModifiedAt: number | null; characterName: string | null; realm: string | null; recordCount: number };
export type SourceView = {
  installation: string | null; sources: { id: string; label: string }[]; selectedSource: string | null;
  status: string; message: string; data: Summary | null; game: GameView; fileModifiedAt: number | null; readAt: number | null;
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
  const game = view.game;
  if (!game || !statuses.includes(game.status) || typeof game.message !== 'string' || !nullableNumber(game.fileModifiedAt)
    || ![game.characterName, game.realm].every(nullableString) || !Number.isSafeInteger(game.recordCount) || game.recordCount < 0 || game.recordCount > 8) return false;
  if (view.data === null) return true;
  const data = view.data;
  return !!data && data.schema === 1 && Number.isSafeInteger(data.recordCount) && data.recordCount >= 0 && data.recordCount <= 300
    && [data.clientVersion, data.clientBuild, data.clientInterface, data.observedCharacter, data.observedRealm, data.observedMap, data.rawWinner, data.lastCapturePhase].every(nullableString)
    && [data.lastRecordAt, data.latestScoreboardRows].every(nullableNumber)
    && ['READABLE_DATA', 'OBSERVED_DIAGNOSTIC_BUILD', 'UNSUPPORTED_CLIENT', 'UNKNOWN_CLIENT'].includes(data.compatibility);
}
function time(value: number | null): string {
  if (value === null) return 'Unknown';
  const date = new Date(value * 1000);
  return Number.isNaN(date.valueOf()) ? 'Unknown' : date.toLocaleString();
}

export function useSavedVariables() {
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
    }).catch(err => { if (!disposed) { resync.current = true; setView(null); setError(errorDetail(err)); } })
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
            setWatchError(errorDetail(err));
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
    } catch (err) { if (mounted.current) { setView(current => current ? { ...current, data: null, fileModifiedAt: null, readAt: null } : null); setError(errorDetail(err)); } }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  }
  return { view, busy, error, watchError, run };
}

export function SavedVariables({ source }: { source: ReturnType<typeof useSavedVariables> }) {
  const privacy = usePrivacy();
  const { view, busy, error, watchError, run } = source;
  const game = view?.game;
  const setup = gameSetup(source);
  const { folderReady, readerFailure } = setup;
  const addonReady = setup.ready;
  const prerequisitesReady = folderReady && !setup.needsSourceSelection;
  const addonTitle = !view && !readerFailure ? 'Checking addon data…' : !folderReady ? 'Waiting for WoW folder'
    : setup.needsSourceSelection ? 'Waiting for WoW account' : addonReady ? 'Addon detected' : setup.title;
  const folderControls = <>
    {view?.installation && <div className="wow-folder-row"><p className="local-path">{privacy.wowPath(view.installation)}</p><button type="button" className="wow-folder-change" title="Change WoW folder" aria-label="Change WoW folder" disabled={busy} onClick={() => void run('choose_forever_installation')}><Icon name="folder" /></button></div>}
    {folderReady && !!view?.sources.length && <div className="source-picker"><label htmlFor="settings-wow-account">WoW account</label>
      <select id="settings-wow-account" value={view.selectedSource ?? ''} disabled={busy}
        onChange={event => { if (event.target.value) void run('select_saved_variables_source', { sourceId: event.target.value }); }}>
        <option value="" disabled>Choose your WoW account</option>
        {view.sources.map(source => <option key={source.id} value={source.id}>{privacy.wowAccount(source.label)}</option>)}
      </select>
    </div>}
    {folderReady && !view?.sources.length && <><p className="saved-data-note">{setup.description}</p><button disabled={busy} onClick={() => void run('saved_variables_status')}>{busy ? 'Checking…' : 'Check for WoW accounts'}</button></>}
    <div className="button-group">
      <button className="button-link" disabled={busy} onClick={() => void run('scan_forever_installations')}>Search for WoW</button>
    </div>
  </>;
  return <section aria-label="Game data settings">
    <section className="settings-section" aria-labelledby="settings-folder">
    <div className="section-heading"><h2 id="settings-folder" tabIndex={-1}>WoW folder</h2>
    </div>
    <div className="section-body"><div className="settings-control-row">
      <div className={`settings-state ${folderReady ? 'is-complete' : readerFailure ? 'is-warning' : 'is-pending'}`}><Icon name={folderReady ? 'check' : 'game'} /><strong>{folderReady ? 'WoW folder selected' : readerFailure ? 'Cannot check WoW folder' : view ? 'Choose your WoW folder' : 'Looking for WoW…'}</strong></div>
      {!view?.installation && <button className="button-primary" disabled={busy} onClick={() => void run('choose_forever_installation')}>Choose WoW folder</button>}
    </div>
    {folderControls}
    {!folderReady && view && <p className="saved-data-note">{setup.description}</p>}
    </div></section>
    <section className="settings-section" aria-labelledby="settings-addon"><div className="section-heading"><h2 id="settings-addon" tabIndex={-1}>rbgs.io addon</h2></div><div className="section-body">
    <div className="settings-control-row"><div className={`settings-state ${addonReady && !busy ? 'is-complete' : readerFailure || folderReady && !!view?.selectedSource && !busy ? 'is-warning' : 'is-pending'}`} role="status"><Icon name={addonReady && !busy ? 'check' : 'game'} /><strong>{addonTitle}</strong></div>
      {prerequisitesReady && !addonReady && !readerFailure && <button disabled={busy} onClick={() => void run('saved_variables_status')}>{busy ? 'Checking…' : 'Check addon data'}</button>}
    </div>
    {readerFailure ? <Feedback title="Cannot read saved game data" detail={error ?? watchError} action={<button disabled={busy} onClick={() => void run('saved_variables_status')}>{busy ? 'Retrying…' : 'Retry addon data'}</button>}>Check the WoW folder and account above. Automatic checks will also retry.</Feedback>
      : prerequisitesReady && !addonReady && <p className="saved-data-note">{setup.description}</p>}
    {addonReady && <><p className="settings-value">{game?.characterName ? `Last saved: ${game.characterName}${game.realm ? ` · ${game.realm}` : ''}` : 'Ready for your first match.'}</p><details className="settings-details"><summary>Saved data and checks</summary><dl className="saved-data-summary">
      <div><dt>File last saved</dt><dd>{time(game!.fileModifiedAt)}</dd></div>
      <div><dt>Recordings in the WoW file</dt><dd>{game!.recordCount}</dd></div>
    </dl><button disabled={busy} onClick={() => void run('saved_variables_status')}>{busy ? 'Checking…' : 'Refresh game data'}</button></details></>}
    </div></section>
    <details className="settings-help"><summary>How saving works</summary><p>WoW saves addon data when you use <code>/reload</code> or log out. Keep the companion running to read it automatically. The character shown comes from saved data; it may not be the one you are playing now.</p></details>
    </section>;
}
