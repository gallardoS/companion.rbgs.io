import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Identity = { name: string; realm: string | null; guid: null };
type Row = { character: Identity; observedTeamKey: string | null; killingBlows: number | null; honorableKills: number | null; deaths: number | null; damage: number | null; healing: number | null };
type Match = { captureId: string; revision: number; character: Identity; map: { profile: string; observedName: string; gameMapId: number | null }; startedAt: string; endedAt: string; winner: string; roster: Row[]; build: string };
type QueueItem = { captureId: string; revision: number; state: string; coverage: string; savedAt: number };
type Queue = { status: string; message: string; snapshots: number; captures: number; conflicts: number; waitingAssignment: number; needsAuth: number; acknowledged?: number; pending?: number; blocked?: number; bytes: number; items: QueueItem[]; sourceStatus: string; sourceMessage: string; unboundSnapshots: number };
type View = { status: string; message: string; matches: Match[]; retainedAttempts: number; queue: Queue | null };
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
function valid(value: unknown): value is View {
  if (!value || typeof value !== 'object') return false;
  const view = value as View;
  return ['READY', 'SELECT_SOURCE', 'MISSING_ADDON', 'NO_SAVED_DATA', 'UNSTABLE_FILE', 'INVALID_DATA', 'SOURCE_UNAVAILABLE'].includes(view.status)
    && text(view.message) && number(view.retainedAttempts) && view.retainedAttempts !== null && view.retainedAttempts <= 8
    && validQueue(view.queue) && Array.isArray(view.matches) && view.matches.length <= (view.queue?.captures ?? view.retainedAttempts)
    && new Set(view.matches.map(m => m?.captureId)).size === view.matches.length
    && (view.status === 'READY' || view.matches.length === 0)
    && view.matches.every(m => m && typeof m.captureId === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(m.captureId)
      && number(m.revision) && m.revision !== null && m.revision >= 1 && m.revision <= 2147483647 && identity(m.character)
      && m.map && m.map.profile === 'DARKSPEAR_DIAGNOSTIC' && text(m.map.observedName) && m.map.gameMapId === 2997
      && text(m.startedAt) && Number.isFinite(Date.parse(m.startedAt)) && text(m.endedAt) && Number.isFinite(Date.parse(m.endedAt))
      && ['0', '1'].includes(m.winner) && typeof m.build === 'string' && /^[0-9]{1,10}$/.test(m.build) && /[1-9]/.test(m.build) && Array.isArray(m.roster) && m.roster.length >= 1 && m.roster.length <= 40
      && m.roster.every(row => row && identity(row.character) && [null, '0', '1'].includes(row.observedTeamKey)
        && [row.killingBlows, row.honorableKills, row.deaths, row.damage, row.healing].every(number)));
}
function stamp(value: string) { return new Date(value).toLocaleString(); }
function team(value: string | null) { return value === '0' ? 'Horde' : value === '1' ? 'Alliance' : 'Unknown'; }
function character(value: Identity) { return `${value.name}${value.realm ? ` · ${value.realm}` : ''}`; }

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

export function MatchHistory() {
  const id = useId();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const refresh = useRef<() => void>(() => {});
  useEffect(() => {
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
        setView(result); setError(null);
      } catch (err) {
        if (!disposed) { setView(null); setError(String(err)); }
      } finally {
        inFlight = false;
        if (!disposed) { setBusy(false); timer = setTimeout(() => void read(), 5000); }
      }
    }
    refresh.current = () => void read();
    void read();
    return () => { disposed = true; clearTimeout(timer); refresh.current = () => {}; };
  }, []);
  return <section className="match-history" aria-labelledby={`${id}-heading`}>
    <div className="section-toolbar"><div><h2 id={`${id}-heading`}>Completed matches</h2>
    <p>Local observations. Matches remain unverified.</p></div>
    <button disabled={busy} onClick={() => refresh.current()}>Refresh match history</button></div>
      {view?.queue && <div className="local-inbox">
        <h3>Local capture queue</h3>
        <p role={view.queue.status === 'READY' ? 'status' : 'alert'}>{view.queue.message}</p>
        <dl className="queue-metrics"><div><dt>Saved revisions</dt><dd>{view.queue.snapshots}</dd></div><div><dt>Received by server</dt><dd>{view.queue.acknowledged ?? 0}</dd></div><div><dt>Pending delivery</dt><dd>{view.queue.pending ?? 0}</dd></div><div><dt>Blocked</dt><dd>{view.queue.blocked ?? 0}</dd></div></dl>
        <p className="saved-data-note">{view.queue.waitingAssignment} waiting for assignment · {view.queue.needsAuth} waiting for account confirmation</p>
        {view.queue.conflicts > 0 && <p role="alert">{view.queue.conflicts} conflicting captures preserved. They are excluded from completed history.</p>}
        {view.queue.unboundSnapshots > 0 && <p>{view.queue.unboundSnapshots} earlier unbound snapshots are preserved separately for assignment recovery.</p>}
        {view.queue.sourceStatus !== 'READY' && <p role="status">{view.queue.sourceMessage} Previously saved captures remain available.</p>}
        {!!view.queue.items.length && <details><summary>Saved revisions</summary><ul>{view.queue.items.map((item, index) => <li key={index}>
          <span className="local-path">{item.captureId}</span> · revision {item.revision} · {item.coverage.toLowerCase()} · {({WAITING_ASSIGNMENT:'Waiting for assignment',NEEDS_AUTH:'Waiting for account confirmation',CONFLICT:'Conflict',PENDING:'Pending delivery',SENDING:'Sending',ACKNOWLEDGED:'Received by server',BLOCKED:'Blocked'})[item.state as 'WAITING_ASSIGNMENT']}
        </li>)}</ul></details>}
      </div>}
    {error ? <p role="alert">{error}</p> : view?.status !== 'READY' ? <p role="status">{view?.message ?? 'Reading match history…'}</p> : <>
      <p>{view.matches.length} completed locally · {view.retainedAttempts} / 8 attempts in the current game file</p>
      {!view.matches.length && !view.queue?.conflicts && view.queue?.status !== 'STORAGE_UNAVAILABLE' && <p>No completed matches yet.</p>}
      {view.matches.map(match => <details className="history-match" key={match.captureId}>
        <summary><strong>{match.map.observedName}</strong><span>{team(match.winner)} won · {match.roster.length} players</span><time dateTime={match.endedAt}>{stamp(match.endedAt)}</time></summary>
        <dl className="saved-data-summary">
          <div><dt>Observed character</dt><dd>{character(match.character)}</dd></div>
          <div><dt>Observed entry</dt><dd>{stamp(match.startedAt)}</dd></div>
          <div><dt>Capture ID / revision / build</dt><dd>{match.captureId} / {match.revision} / {match.build}</dd></div>
        </dl>
        <Scoreboard match={match} />
      </details>)}
    </>}
  </section>;
}
