import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './Navigation';
import type { ClosedTest } from './testHistory';
import { TestMatchProgress } from './TestMatchProgress';
import { errorDetail, Feedback } from './Feedback';

type Assignment = { captureId: string; assignmentId: string; character: { name: string; realm: string | null }; armUntil: string; codeIssued: boolean; cancelled: boolean; command: string | null };
export type AssignmentSummary = { captureIds: string[]; error: boolean; closedTests: ClosedTest[] };
const commandPattern = /^\/rbgs match RB2:[0-9a-f-]{36}:[0-9A-HJKMNP-TV-Z]{16}$/i;
function timeRemaining(deadline: string, now: number) {
  const seconds = Math.max(0, Math.ceil((Date.parse(deadline) - now) / 1000));
  if (seconds >= 86400) return `${Math.floor(seconds / 86400)}d ${Math.floor(seconds % 86400 / 3600)}h`;
  if (seconds >= 3600) return `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)} min`;
  if (seconds >= 60) return `${Math.ceil(seconds / 60)} min`;
  return `${seconds} sec`;
}

export function CaptureAssignments({ accountId, enabled = true, onSummary, activityVersion = 0, focusCaptureId = null }: { accountId: string; enabled?: boolean; onSummary?: (summary: AssignmentSummary) => void; activityVersion?: number; focusCaptureId?: string | null }) {
  const [rows, setRows] = useState<Assignment[]>([]);
  const [loading, setLoading] = useState(true);
  const [issuingId, setIssuingId] = useState<string | null>(null);
  const issuing = issuingId !== null;
  const [sending, setSending] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [issueError, setIssueError] = useState<{ captureId: string; detail: string } | null>(null);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [syncMessage, setSyncMessage] = useState('');
  const [copyFallback, setCopyFallback] = useState<string | null>(null);
  const preferenceKey = `rbgs.auto-uploads.v1.${accountId}`;
  const [uploadPreference, setUploadPreference] = useState<{ enabled: boolean; error: 'read' | 'write' | null }>(() => {
    try {
      const saved = localStorage.getItem(preferenceKey);
      if (saved !== null && saved !== 'on' && saved !== 'paused') return { enabled: false, error: 'read' };
      return { enabled: saved !== 'paused', error: null };
    }
    catch { return { enabled: false, error: 'read' }; }
  });
  const autoSync = uploadPreference.enabled;
  const [copied, setCopied] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const syncing = useRef(false);
  const issuingRef = useRef(false);
  const mounted = useRef(false);
  const loadEpoch = useRef(0);
  const lastFocused = useRef<string | null>(null);
  const allowed = useRef(enabled);
  allowed.current = enabled;
  const busy = issuing || sending;
  const visibleRows = rows.filter(row => !row.cancelled && Date.parse(row.armUntil) > now);


  function changeAutoSync(value: boolean) {
    try { localStorage.setItem(preferenceKey, value ? 'on' : 'paused'); setUploadPreference({ enabled: value, error: null }); }
    catch { setUploadPreference({ enabled: value, error: 'write' }); }
  }

  useEffect(() => {
    mounted.current = true;
    const updateNow = () => setNow(Date.now());
    const timer = window.setInterval(updateNow, 1000);
    window.addEventListener('focus', updateNow);
    return () => { mounted.current = false; ++loadEpoch.current; window.clearInterval(timer); window.removeEventListener('focus', updateNow); };
  }, []);

  const load = useCallback(async () => {
    if (!allowed.current) return;
    const epoch = ++loadEpoch.current;
    setLoading(true);
    try {
      const result = await invoke<Assignment[]>('list_capture_assignments');
      if (!Array.isArray(result) || result.length > 50 || !result.every(row => row && typeof row.captureId === 'string' && /^[0-9a-f-]{36}$/i.test(row.captureId) && row.character && typeof row.character.name === 'string' && (row.character.realm === null || typeof row.character.realm === 'string') && typeof row.codeIssued === 'boolean' && typeof row.cancelled === 'boolean' && Number.isFinite(Date.parse(row.armUntil)) && (row.command === null || typeof row.command === 'string' && commandPattern.test(row.command)))) throw Error('Invalid assignment list');
      if (!mounted.current || epoch !== loadEpoch.current || !allowed.current) return;
      setRows(result); setLoaded(true); setLoadError(null);
    } catch (err) {
      if (mounted.current && epoch === loadEpoch.current && allowed.current) { setLoadError(errorDetail(err)); setLoaded(true); }
    } finally {
      if (mounted.current && epoch === loadEpoch.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (enabled) void load();
    else { ++loadEpoch.current; setLoading(false); }
  }, [enabled, load, activityVersion]);
  useEffect(() => {
    if (!enabled) return;
    const refresh = () => void load();
    const timer = window.setInterval(refresh, 15000);
    window.addEventListener('focus', refresh);
    return () => { window.clearInterval(timer); window.removeEventListener('focus', refresh); };
  }, [enabled, load]);
  useEffect(() => {
    if (!focusCaptureId) { lastFocused.current = null; return; }
    if (lastFocused.current === focusCaptureId || !rows.some(row => row.captureId === focusCaptureId)) return;
    const card = document.getElementById(`assignment-${focusCaptureId}`);
    if (!card) return;
    card.focus({ preventScroll: true });
    card.scrollIntoView({ block: 'nearest' });
    lastFocused.current = focusCaptureId;
  }, [focusCaptureId, rows]);
  useEffect(() => {
    if (!autoSync || !enabled) return;
    const timer = window.setInterval(() => void sync(), 15000);
    return () => window.clearInterval(timer);
  }, [autoSync, enabled]);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(null), 2500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  useEffect(() => {
    if (loaded) onSummary?.({ captureIds: rows.filter(row => !row.cancelled && Date.parse(row.armUntil) > now).map(row => row.captureId), error: loadError !== null, closedTests: rows.filter(row => row.cancelled || Date.parse(row.armUntil) <= now).map(({ captureId, character, armUntil, cancelled }) => ({ captureId, character, armUntil, cancelled })) });
  }, [rows, loaded, loadError, now, onSummary]);

  async function issue(row: Assignment) {
    if (!allowed.current || syncing.current || issuingRef.current) return;
    issuingRef.current = true; setIssuingId(row.captureId);
    try {
      const command = await invoke<string>('issue_capture_assignment', { captureId: row.captureId });
      if (typeof command !== 'string' || !commandPattern.test(command)) throw Error('Invalid assignment command');
      if (!mounted.current || !allowed.current) return;
      await load();
      if (!mounted.current || !allowed.current) return;
      setRows(current => current.map(item => item.captureId === row.captureId ? { ...item, command, codeIssued: true } : item));
      setIssueError(null);
    } catch (err) { if (mounted.current && allowed.current) setIssueError({ captureId: row.captureId, detail: errorDetail(err) }); }
    finally { issuingRef.current = false; if (mounted.current) setIssuingId(null); }
  }

  async function sync(manual = false) {
    if (!allowed.current || syncing.current || issuingRef.current) return;
    syncing.current = true; setSending(true);
    try {
      const result = await invoke<{ checked: number; acknowledged: number; waiting: number; message: string }>('sync_captures');
      if (!result || ![result.checked, result.acknowledged, result.waiting].every(n => Number.isSafeInteger(n) && n >= 0 && n <= 3) || typeof result.message !== 'string') throw Error('Invalid synchronization response');
      if (mounted.current && allowed.current) {
        setUploadError(null);
        if (manual) setSyncMessage(result.checked === 0 ? 'Nothing was sent during this check. See each match for its upload status.' : `This check: ${result.acknowledged} confirmed · ${result.waiting} waiting.`);
      }
    } catch (err) { if (mounted.current && allowed.current) { setUploadError(errorDetail(err)); setSyncMessage(''); } }
    finally { syncing.current = false; if (mounted.current) setSending(false); }
  }

  async function copy(row: Assignment) {
    if (!allowed.current || !row.command || Date.parse(row.armUntil) <= Date.now()) return;
    try { await navigator.clipboard.writeText(row.command); setCopied(row.captureId); setCopyFallback(null); }
    catch { document.getElementById(`command-${row.captureId}`)?.focus(); setCopyFallback(row.captureId); }
  }

  const uploadFeedback = enabled && uploadError !== null && <Feedback title="Cannot check uploads" detail={uploadError} action={<button disabled={busy} onClick={() => void sync(true)}><Icon name="reload" />{sending ? 'Retrying…' : 'Retry uploads'}</button>}>{autoSync ? 'Saved data is still on this PC. Automatic uploads will retry.' : 'Saved data is still on this PC. Automatic uploads are paused.'}</Feedback>;
  const preferenceFeedback = uploadPreference.error !== null && <p className="status-message" role="alert">{uploadPreference.error === 'read' ? 'Cannot read your saved upload preference. Automatic uploads are paused until you choose to resume.' : 'Could not save your upload preference. It applies until you close the companion.'}</p>;
 return <section className="capture-assignments"><div className="section-toolbar"><div><h2>Test matches</h2><p>Assigned captures go to review. No rating is applied.</p></div><div className="button-group"><button disabled={busy || loading} onClick={()=>void load()}>Check for test matches</button><button className="button-primary" disabled={busy} onClick={()=>void sync(true)}>{sending ? 'Checking uploads…' : 'Check uploads now'}</button></div></div>
 <label className="capture-auto-sync"><span><strong>Automatic uploads</strong><span>Upload saved data for your assigned matches. This preference is remembered on this PC.</span></span><input type="checkbox" checked={autoSync} onChange={e=>changeAutoSync(e.target.checked)} aria-label="Automatically upload data for assigned matches" /></label>{syncMessage && <p className="status-message" role="status">{syncMessage}</p>}
    {!autoSync && <div className="uploads-paused" role="status"><span>Automatic uploads paused{sending ? ' · Current check may finish' : ''}</span><button onClick={() => changeAutoSync(true)}>Resume uploads</button></div>}
    {preferenceFeedback}
    {uploadFeedback}
    {enabled && loadError !== null && <Feedback title="Cannot load test matches" detail={loadError} action={<button disabled={busy || loading} onClick={() => void load()}><Icon name="reload" />{loading ? 'Retrying…' : 'Retry tests'}</button>}>{visibleRows.length ? 'Showing the last loaded tests. Retrying automatically.' : 'Retrying automatically. You can also try again now.'}</Feedback>}
    {enabled && loading && !loaded && <p role="status">Loading your test matches…</p>}
 {visibleRows.map(row=><article className={`history-match${focusCaptureId === row.captureId ? ' is-selected' : ''}`} id={`assignment-${row.captureId}`} tabIndex={-1} key={row.captureId}><h3>{row.character.name} · Darkspear</h3><details className="assignment-deadline"><summary><strong className={Date.parse(row.armUntil) - now < 60000 ? 'text-warning' : undefined}>Expires in {timeRemaining(row.armUntil, now)}</strong></summary><p>Use by <time dateTime={row.armUntil}>{new Date(row.armUntil).toLocaleString()}</time></p></details>
 { !row.cancelled && Date.parse(row.armUntil) > Date.now() && <>
   <TestMatchProgress step={row.codeIssued || row.command ? 2 : 1} />
   <div className="test-next-step"><strong>{row.command ? 'Waiting for saved results' : row.codeIssued ? 'Check your command in WoW' : 'Get your match command'}</strong>
     <p>{row.command ? 'Paste the command in WoW chat before entering the battleground. The companion receives results when you save them at the end; it cannot confirm when the command is used.' : row.codeIssued ? 'If you already used the command, save your results when the battleground ends.' : 'Then paste it in WoW chat before entering the battleground.'}</p>
   </div>
 </>}

 <button disabled={busy||row.cancelled||Date.parse(row.armUntil)<=Date.now()||(row.codeIssued&&!row.command)} onClick={()=>void issue(row)}>{issuingId === row.captureId ? 'Getting command…' : row.command ? 'Show saved command' : 'Get match command'}</button>
        {issueError?.captureId === row.captureId && !row.command && !(row.codeIssued && !row.command) && <Feedback title="Cannot get your command" detail={issueError.detail}>Try Get match command again. If a command was already issued, check the rbgs.io addon in WoW before requesting a new test.</Feedback>}
 {row.command&&!row.cancelled&&<><label htmlFor={`command-${row.captureId}`}>Match command to paste in WoW chat</label><div className="command-field">
          <input id={`command-${row.captureId}`} readOnly value="/rbgs match ••••••••••••••••••••" onFocus={event => event.target.select()} onCopy={event => { event.preventDefault(); if (!allowed.current || Date.parse(row.armUntil) <= Date.now()) return; event.clipboardData.setData('text/plain', row.command!); setCopied(row.captureId); setCopyFallback(null); }} />
          <button className="button-primary command-copy" disabled={Date.parse(row.armUntil) <= Date.now()} onClick={() => void copy(row)}><Icon name={copied === row.captureId ? 'check' : 'copy'} />{copied === row.captureId ? 'Copied' : 'Copy command'}</button>
          <span className="visually-hidden" role="status">{copied === row.captureId ? 'Command copied.' : ''}</span>
 </div>{copyFallback === row.captureId && <p className="status-message" role="status">Press Ctrl+C to copy the selected command.</p>}</>}
 {row.codeIssued&&!row.command&&<p>The code was already issued elsewhere or its response was lost. Recover from the addon if armed; otherwise request a fresh assignment.</p>}
        <p className="saved-data-note">At the final scoreboard, click <strong>rbgs.io: Save results</strong> or use <code>/reload</code>. The companion will then upload the saved results{autoSync ? ' automatically' : ' when you resume uploads'}.</p>
 </article>)}</section>;
}
