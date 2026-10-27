import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './Navigation';
import { useHiddenTests, type ClosedTest } from './testHistory';
import { errorDetail, Feedback } from './Feedback';
import { TestMatchProgress } from './TestMatchProgress';

type Identity = { name: string; realm: string | null; guid: null };
type Row = { character: Identity; observedTeamKey: string | null; killingBlows: number | null; honorableKills: number | null; deaths: number | null; damage: number | null; healing: number | null };
type Match = { captureId: string; revision: number; character: Identity; map: { profile: string; observedName: string; gameMapId: number | null }; startedAt: string; endedAt: string; winner: string; roster: Row[]; build: string };
type QueueItem = { captureId: string; revision: number; state: string; coverage: string; savedAt: number };
type Queue = { status: string; message: string; snapshots: number; captures: number; conflicts: number; waitingAssignment: number; needsAuth: number; acknowledged?: number; pending?: number; blocked?: number; bytes: number; items: QueueItem[]; sourceStatus: string; sourceMessage: string; unboundSnapshots: number };
export type HistoryView = { status: string; message: string; matches: Match[]; retainedAttempts: number; queue: Queue | null };
const text = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && v.length <= 512;
const number = (v: unknown): v is number | null => v === null || typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const identity = (v: Identity) => !!v && text(v.name) && (v.realm === null || text(v.realm)) && v.guid === null;
function validQueue(queue: Queue | null) {
  return queue === null || !!queue && ['READY', 'STORAGE_FULL', 'STORAGE_UNAVAILABLE', 'UNSUPPORTED_STORAGE', 'INVALID_DATA'].includes(queue.status)
    && text(queue.message) && text(queue.sourceStatus) && text(queue.sourceMessage)
    && [queue.snapshots, queue.captures, queue.conflicts, queue.waitingAssignment, queue.needsAuth, queue.unboundSnapshots].every(n => number(n) && n !== null && n <= 100)
    && [queue.acknowledged, queue.pending, queue.blocked].every(n => n === undefined || number(n) && n !== null && n <= 100)
    && queue.captures <= queue.snapshots && queue.conflicts <= queue.captures && queue.waitingAssignment + queue.needsAuth <= queue.snapshots
    && number(queue.bytes) && queue.bytes !== null && queue.bytes <= 25 * 1024 * 1024
    && Array.isArray(queue.items) && queue.items.length === queue.snapshots && queue.items.every(item => item && text(item.captureId)
      && /^[0-9a-f-]{36}$/i.test(item.captureId) && number(item.revision) && item.revision >= 1 && item.revision <= 2147483647
      && ['WAITING_ASSIGNMENT', 'NEEDS_AUTH', 'CONFLICT', 'PENDING', 'SENDING', 'ACKNOWLEDGED', 'BLOCKED'].includes(item.state) && ['COMPLETE', 'INCOMPLETE'].includes(item.coverage) && number(item.savedAt));
}
function valid(value: unknown): value is HistoryView {
  if (!value || typeof value !== 'object') return false;
  const view = value as HistoryView;
  return ['READY', 'SELECT_SOURCE', 'MISSING_ADDON', 'NO_SAVED_DATA', 'UNSTABLE_FILE', 'INVALID_DATA', 'SOURCE_UNAVAILABLE'].includes(view.status)
    && text(view.message) && number(view.retainedAttempts) && view.retainedAttempts !== null && view.retainedAttempts <= 8
    && validQueue(view.queue) && Array.isArray(view.matches) && view.matches.length <= (view.queue?.captures ?? view.retainedAttempts)
    && new Set(view.matches.map(m => m?.captureId)).size === view.matches.length
    && (view.status === 'READY' || view.matches.length === 0)
    && view.matches.every(m => m && typeof m.captureId === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(m.captureId)
      && number(m.revision) && m.revision !== null && m.revision >= 1 && m.revision <= 2147483647 && identity(m.character)
      && m.map && (m.map.profile === 'DARKSPEAR_DIAGNOSTIC' && m.map.gameMapId === 2997 || m.map.profile === 'TBC_ANNIVERSARY_WSG_TEST' && m.map.gameMapId === 489) && text(m.map.observedName)
      && text(m.startedAt) && Number.isFinite(Date.parse(m.startedAt)) && text(m.endedAt) && Number.isFinite(Date.parse(m.endedAt))
      && ['0', '1'].includes(m.winner) && typeof m.build === 'string' && /^[0-9]{1,10}$/.test(m.build) && /[1-9]/.test(m.build) && Array.isArray(m.roster) && m.roster.length >= 1 && m.roster.length <= 40
      && m.roster.every(row => row && identity(row.character) && [null, '0', '1'].includes(row.observedTeamKey)
        && [row.killingBlows, row.honorableKills, row.deaths, row.damage, row.healing].every(number)));
}
function stamp(value: string) { return new Date(value).toLocaleString(); }
function shortStamp(value: string) {
  const date = new Date(value);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const day = date.toDateString() === today.toDateString() ? 'Today'
    : date.toDateString() === yesterday.toDateString() ? 'Yesterday'
    : date.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', ...(date.getFullYear() !== today.getFullYear() ? { year: 'numeric' as const } : {}) });
  return `${day} · ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
}
function team(value: string | null) { return value === '0' ? 'Horde' : value === '1' ? 'Alliance' : 'Unknown'; }
function character(value: Identity) { return `${value.name}${value.realm ? ` · ${value.realm}` : ''}`; }
function matchResult(match: Match): 'victory' | 'defeat' | null {
  const name = (value: string) => value.normalize('NFC').trim().toLowerCase();
  const realm = (value: string) => name(value).replace(/\s+/g, '');
  const ownRows = match.roster.filter(row => name(row.character.name) === name(match.character.name)
    && (!row.character.realm || !match.character.realm || realm(row.character.realm) === realm(match.character.realm)));
  if (ownRows.length !== 1 || ownRows[0].observedTeamKey === null) return null;
  return ownRows[0].observedTeamKey === match.winner ? 'victory' : 'defeat';
}

const scoreboardColumns = [
  { key: 'character', label: 'Character', numeric: false },
  { key: 'team', label: 'Team', numeric: false },
  { key: 'killingBlows', label: 'KB', description: 'Killing blows', numeric: true },
  { key: 'honorableKills', label: 'HK', description: 'Honorable kills', numeric: true },
  { key: 'deaths', label: 'Deaths', numeric: true },
  { key: 'damage', label: 'Damage', numeric: true },
  { key: 'healing', label: 'Healing', numeric: true },
] as const;
type ScoreboardColumn = typeof scoreboardColumns[number]['key'];
const scoreboardNames = new Intl.Collator('en', { sensitivity: 'base', numeric: true });

function Scoreboard({ match }: { match: Match }) {
  const captionId = useId();
  const [sort, setSort] = useState<{ key: ScoreboardColumn; descending: boolean } | null>(null);
  const rows = match.roster.map((row, index) => ({ row, index }));
  if (sort) {
    const value = (row: Row) => sort.key === 'character' ? character(row.character)
      : sort.key === 'team' ? row.observedTeamKey === null ? null : team(row.observedTeamKey) : row[sort.key];
    rows.sort((a, b) => {
      const left = value(a.row), right = value(b.row);
      if (left === null || right === null) return left === right ? a.index - b.index : left === null ? 1 : -1;
      const order = typeof left === 'number' && typeof right === 'number' ? left - right : scoreboardNames.compare(String(left), String(right));
      return (sort.descending ? -order : order) || a.index - b.index;
    });
  }
  const activeColumn = scoreboardColumns.find(column => column.key === sort?.key);
  return <div className="scoreboard"><p className="scoreboard-caption" id={captionId}>Saved final scoreboard</p><div className="scoreboard-scroll" role="region" aria-label={`Scoreboard for ${match.map.observedName}`} tabIndex={0}>
    <p className="visually-hidden" role="status">{sort && activeColumn ? `Sorted by ${activeColumn.label}, ${sort.descending ? 'descending' : 'ascending'}.` : ''}</p>
    <table aria-labelledby={captionId}><thead><tr>{scoreboardColumns.map(column => {
      const active = sort?.key === column.key;
      const descending = active ? !sort.descending : column.numeric;
      const description = 'description' in column ? column.description : column.label;
      return <th scope="col" key={column.key} aria-sort={active ? sort.descending ? 'descending' : 'ascending' : undefined}>
        <button className="scoreboard-sort" aria-label={`Sort by ${description}`} title={`Sort by ${description}, ${column.numeric ? descending ? 'highest first' : 'lowest first' : descending ? 'Z to A' : 'A to Z'}`} onClick={() => setSort({ key: column.key, descending })}>{column.label}<span aria-hidden="true">{active ? sort.descending ? '↓' : '↑' : '↕'}</span></button>
      </th>;
    })}</tr></thead>
      <tbody>{rows.map(({ row, index }) => <tr key={index}><th scope="row"><span className="scoreboard-name">{row.character.name}</span>{row.character.realm && <span className="scoreboard-realm">{row.character.realm}</span>}</th><td className={row.observedTeamKey === '0' ? 'team-horde' : row.observedTeamKey === '1' ? 'team-alliance' : undefined}>{team(row.observedTeamKey)}</td>
        {[row.killingBlows, row.honorableKills, row.deaths, row.damage, row.healing].map((value, column) => <td key={column}>{value === null ? 'Unknown' : value.toLocaleString()}</td>)}
      </tr>)}</tbody></table>
  </div></div>;
}

export function useMatchHistory(scope: string | null) {
  const [snapshot, setSnapshot] = useState<{ scope: string | null; view: HistoryView | null; error: string | null }>({ scope: null, view: null, error: null });
  const [busy, setBusy] = useState(false);
  const refresh = useRef<() => void>(() => {});
  useEffect(() => {
    if (scope === null) { refresh.current = () => {}; setBusy(false); return; }
    let disposed = false;
    let inFlight = false;
    let timer: ReturnType<typeof setTimeout>;
    async function read() {
      if (disposed || inFlight) return;
      clearTimeout(timer);
      inFlight = true; setBusy(true);
      try {
        const result = await invoke<unknown>('match_history');
        if (disposed) return;
        if (!valid(result)) throw new Error('Invalid match-history response');
        setSnapshot({ scope, view: result, error: null });
      } catch (err) {
        if (!disposed) setSnapshot(current => ({ scope, view: current.scope === scope ? current.view : null, error: errorDetail(err) }));
      } finally {
        inFlight = false;
        if (!disposed) { setBusy(false); timer = setTimeout(() => void read(), 5000); }
      }
    }
    refresh.current = () => void read();
    void read();
    const onFocus = () => void read();
    window.addEventListener('focus', onFocus);
    return () => { disposed = true; clearTimeout(timer); refresh.current = () => {}; window.removeEventListener('focus', onFocus); };
  }, [scope]);
  return { view: snapshot.scope === scope ? snapshot.view : null, error: snapshot.scope === scope ? snapshot.error : null, busy, refresh: () => refresh.current() };
}

const deliveryLabels: Record<string, string> = {
  WAITING_ASSIGNMENT: 'No match assigned', NEEDS_AUTH: 'Account check needed', CONFLICT: 'Data does not match',
  PENDING: 'Waiting to upload', SENDING: 'Uploading', ACKNOWLEDGED: 'Uploaded', BLOCKED: 'Upload blocked',
};

function deliveryState(match: Match, queue: Queue | null) {
  return queue?.items.find(item => item.captureId === match.captureId && item.revision === match.revision)?.state;
}
const deliveryHelp: Record<string, string> = {
  WAITING_ASSIGNMENT: 'This recording has no matching test assignment. It stays saved on this PC.',
  NEEDS_AUTH: 'Check your account connection and match permissions in Settings to continue uploading.',
  CONFLICT: 'Different data was found for the same recording. It stays saved for inspection.',
  BLOCKED: 'rbgs.io did not accept this upload. The recording stays saved on this PC.',
  PENDING: 'Waiting for the next upload attempt. Keep the companion running with automatic uploads enabled.',
  SENDING: 'Sending this saved version to rbgs.io.',
};

export function DeliveryBadge({ match, queue }: { match: Match; queue: Queue | null }) {
  const state = deliveryState(match, queue);
  const tone = state === 'ACKNOWLEDGED' ? 'success' : state && ['BLOCKED', 'CONFLICT', 'NEEDS_AUTH', 'WAITING_ASSIGNMENT'].includes(state) ? 'warning' : 'neutral';
  return <span className={`state-label state-${tone}`} title={state ? deliveryHelp[state] : undefined}>{state ? deliveryLabels[state] : 'Saved on this PC'}</span>;
}

export function MatchHistory({ history, tests, accountId, sourceSelected, onSettings, onUploads }: { history: ReturnType<typeof useMatchHistory>; tests: ClosedTest[]; accountId: string; sourceSelected: boolean; onSettings: () => void; onUploads: () => void }) {
  const id = useId();
  const { view, error, busy, refresh } = history;
  const [limit, setLimit] = useState(5);
  const hiddenTests = useHiddenTests(accountId);
  const closedTestsRef = useRef<HTMLDetailsElement>(null);
  const undoRef = useRef<HTMLButtonElement>(null);
  const previousHidden = useRef<string | null>(null);
  useEffect(() => {
    if (hiddenTests.lastHidden) undoRef.current?.focus();
    else if (previousHidden.current) closedTestsRef.current?.querySelector('summary')?.focus();
    previousHidden.current = hiddenTests.lastHidden;
  }, [hiddenTests.lastHidden]);
  const recordedIds = new Set(view?.matches.map(match => match.captureId));
  const unrecordedTests = tests.filter(test => !recordedIds.has(test.captureId));
  const hiddenCount = unrecordedTests.filter(test => hiddenTests.hidden.has(test.captureId)).length;
  const matches = [...(view?.matches ?? [])].sort((a, b) => Date.parse(b.endedAt) - Date.parse(a.endedAt) || a.captureId.localeCompare(b.captureId));
  const closedTests = unrecordedTests.filter(test => !hiddenTests.hidden.has(test.captureId))
    .sort((a, b) => Date.parse(b.armUntil) - Date.parse(a.armUntil) || a.captureId.localeCompare(b.captureId));
  const queue = view?.queue;
  const uploadIssue = !!queue && (queue.status !== 'READY' || queue.conflicts > 0 || queue.waitingAssignment > 0 || queue.needsAuth > 0 || (queue.blocked ?? 0) > 0);
  const uploadProblems = queue ? [
    queue.status !== 'READY' ? 'Local match storage needs attention.' : null,
    queue.conflicts > 0 ? 'Conflicting recordings are preserved but excluded from the history.' : null,
    (queue.blocked ?? 0) > 0 ? 'Some uploads were not accepted by rbgs.io.' : null,
    queue.waitingAssignment > 0 ? 'Some saved data has no matching test assignment.' : null,
    queue.needsAuth > 0 ? 'Check your account connection and permissions in Settings.' : null,
  ].filter(Boolean).join(' ') : '';
  return <section className="match-history" aria-labelledby={`${id}-heading`}>
    <div className="section-toolbar"><div><h2 id={`${id}-heading`}>Recorded matches{matches.length > 0 && <span className="section-count">{matches.length}</span>}</h2>
    <p>Uploaded means saved by rbgs.io. Results still await verification.</p></div>
    <div className="button-group"><button className="button-link" onClick={onUploads}>Upload options<Icon name="arrow" /></button>{error && <button disabled={busy || !sourceSelected} onClick={refresh}><Icon name="reload" />Retry</button>}</div></div>
    {hiddenTests.error && <p className="status-message" role="alert">Could not save your hidden tests on this PC. Try again.</p>}
    {error && <Feedback title="Cannot refresh your matches" detail={error}>{view ? 'Showing the last loaded matches. Retrying automatically.' : 'Your matches could not be loaded. Retrying automatically.'}</Feedback>}
    {uploadIssue && <Feedback title="Some saved data needs attention" action={queue!.needsAuth > 0 ? <button onClick={() => onSettings()}>Check account<Icon name="arrow" /></button> : undefined}>{uploadProblems} Your saved data is kept on this PC.</Feedback>}
    {!error && sourceSelected && view?.status !== 'READY' && <p role="status">{view?.message ?? 'Loading saved matches…'}</p>}
    {!matches.length && view?.status === 'READY' && !view.queue?.conflicts && view.queue?.status !== 'STORAGE_UNAVAILABLE' && <div className="compact-empty"><h3>No match history yet</h3><p>Your matches appear here after you save results in WoW.</p></div>}
      {matches.slice(0, limit).map(match => {
        const result = matchResult(match);
        const state = deliveryState(match, queue ?? null);
        return <details className={`history-match${result ? ` is-${result}` : ''}`} key={`match-${match.captureId}`}>
        <summary><span className="match-identity"><strong>{match.map.observedName}</strong><span className="match-meta"><span>{character(match.character)}<span className={`match-outcome${result ? ` is-${result}` : ''}`}>{result === 'victory' ? 'Victory' : result === 'defeat' ? 'Defeat' : `${team(match.winner)} won`}</span></span><time dateTime={match.endedAt} title={stamp(match.endedAt)}>{shortStamp(match.endedAt)}</time></span></span><DeliveryBadge match={match} queue={view?.queue ?? null} /></summary>
        <TestMatchProgress step={3} uploaded={state === 'ACKNOWLEDGED'} />
        <div className="test-next-step"><strong>{state === 'ACKNOWLEDGED' ? 'Results uploaded' : state === 'SENDING' ? 'Uploading saved results' : state ? deliveryLabels[state] : 'Final results saved on this PC'}</strong><p>{state === 'ACKNOWLEDGED' ? 'rbgs.io has received this saved version. Uploading does not verify the result.' : state && deliveryHelp[state] ? deliveryHelp[state] : 'Check Upload options to upload results for an assigned test.'}</p></div>
        <p className="saved-data-note">{match.roster.length} {match.roster.length === 1 ? 'player' : 'players'} in the saved scoreboard.</p>
        <Scoreboard match={match} />
        <details className="diagnostics"><summary>Technical details</summary><dl className="saved-data-summary">
          <div><dt>Saved start time</dt><dd>{stamp(match.startedAt)}</dd></div>
          <div><dt>Match data ID / version / game build</dt><dd>{match.captureId} / {match.revision} / {match.build}</dd></div>
        </dl></details>
      </details>;
      })}
    {matches.length > limit && <button className="button-link history-pagination" onClick={() => setLimit(current => current + 5)}>Show older ({matches.length - limit})</button>}
    {limit > 5 && <button className="button-link history-pagination" onClick={() => setLimit(5)}>Show fewer</button>}
    {hiddenTests.lastHidden && <div className="history-undo"><span role="status">Test hidden on this PC.</span><button ref={undoRef} className="button-link" onClick={hiddenTests.undo}>Undo hide</button></div>}
    {(closedTests.length > 0 || hiddenCount > 0) && <details className="closed-tests" ref={closedTestsRef}><summary>Closed tests <span>({closedTests.length})</span></summary>
      {closedTests.map(test => <article className="history-test" key={test.captureId}>
        <div className="match-identity"><strong>{test.character.name}</strong><span className="match-meta"><span>{test.character.realm ? `${test.character.realm} · ` : ''}{test.expectedMapProfile === 'TBC_ANNIVERSARY_WSG_TEST' ? 'TBC Anniversary / WSG · Temporary test' : 'Darkspear'}</span><time dateTime={test.armUntil} title={`Command deadline: ${stamp(test.armUntil)}`}>{shortStamp(test.armUntil)}</time></span></div>
        <span className="state-label state-neutral">{test.cancelled ? 'Cancelled' : 'Expired'}</span>
        <button className="history-test-hide" title="Hide test on this PC" aria-label={`Hide ${test.cancelled ? 'cancelled' : 'expired'} test for ${test.character.name}`} onClick={() => hiddenTests.hide(test.captureId)}><Icon name="close" /></button>
      </article>)}
      {hiddenCount > 0 && <button className="button-link hidden-tests-action" onClick={hiddenTests.showAll}>Show hidden tests ({hiddenCount})</button>}
    </details>}
    {view?.queue && <details className={`local-inbox${uploadIssue ? ' has-issues' : ''}`}><summary>{uploadIssue ? 'Upload issues' : 'Upload details'}<span> · {view.queue.snapshots} saved versions</span></summary>
    {view?.queue && <div className="delivery-summary" aria-label="Upload totals for saved versions, including updates to the same match">
      <span><strong>{view.queue.acknowledged ?? '—'}</strong> uploaded</span><span><strong>{view.queue.pending ?? '—'}</strong> waiting</span><span><strong>{view.queue.blocked ?? '—'}</strong> blocked</span><span className="muted">Saved versions, including updates to the same match</span>
    </div>}
      <p>{view.queue.message}</p>
      <p className="saved-data-note">{view.queue.waitingAssignment} without a match assignment · {view.queue.needsAuth} need an account check</p>
      {view.queue.unboundSnapshots > 0 && <p>{view.queue.unboundSnapshots} older saved versions are kept separately because their WoW folder and account are unknown.</p>}
      {view.queue.sourceStatus !== 'READY' && <p role="status">{view.queue.sourceMessage} Previously saved matches are still available.</p>}
      <p>{view.matches.length} completed matches saved on this PC · {view.retainedAttempts} / 8 attempts in the current game file</p>
      {!!view.queue.items.length && <ul>{view.queue.items.map(item => <li key={`${item.captureId}/${item.revision}`}><span className="local-path">{item.captureId}</span> · version {item.revision} · {item.coverage.toLowerCase()} · {deliveryLabels[item.state]}</li>)}</ul>}
    </details>}
  </section>;
}
