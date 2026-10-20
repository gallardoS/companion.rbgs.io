import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import { Icon } from './Navigation';
type Assignment={captureId:string;assignmentId:string;character:{name:string;realm:string|null};armUntil:string;codeIssued:boolean;cancelled:boolean;command:string|null};
export function CaptureAssignments({ accountId }: { accountId: string }){
 const [rows,setRows]=useState<Assignment[]>([]);const [busy,setBusy]=useState(false);const [message,setMessage]=useState('Loading controlled assignments…');
 const syncing=useRef(false);const mounted=useRef(true);
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

 const [copied,setCopied]=useState<string|null>(null);
 const [copyFallback,setCopyFallback]=useState<string|null>(null);
  function changeAutoSync(value: boolean) {
    try { localStorage.setItem(preferenceKey, value ? 'on' : 'paused'); setUploadPreference({ enabled: value, error: null }); }
    catch { setUploadPreference({ enabled: value, error: 'write' }); }
  }
 async function load(){try{const result=await invoke<Assignment[]>('list_capture_assignments');if(!Array.isArray(result)||result.length>50||!result.every(r=>r&&typeof r.captureId==='string'&&/^[0-9a-f-]{36}$/i.test(r.captureId)&&r.character&&typeof r.character.name==='string'&&typeof r.codeIssued==='boolean'&&typeof r.cancelled==='boolean'&&Number.isFinite(Date.parse(r.armUntil))))throw Error('Invalid assignment list');setRows(result);setMessage(result.length?'Controlled tests. Captures go to review; no rating is applied.':'No controlled assignments are available for this account.');}catch(e){setRows([]);setMessage(String(e));}}
 useEffect(()=>{mounted.current=true;void load();return()=>{mounted.current=false;};},[]);
 useEffect(()=>{if(!autoSync)return;const timer=window.setInterval(()=>void sync(),15000);return()=>window.clearInterval(timer);},[autoSync]);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(null), 2500);
    return () => window.clearTimeout(timer);
  }, [copied]);
  async function copy(row: Assignment) {
    if (!mounted.current || !row.command || Date.parse(row.armUntil) <= Date.now()) return;
    try { await navigator.clipboard.writeText(row.command); setCopied(row.captureId); setCopyFallback(null); }
    catch { document.getElementById(`command-${row.captureId}`)?.focus(); setCopyFallback(row.captureId); }
  }
 async function issue(row:Assignment){if(busy)return;setBusy(true);try{const command=await invoke<string>('issue_capture_assignment',{captureId:row.captureId});if(typeof command!=='string'||!/^\/rbgs match RB2:[0-9a-f-]{36}:[0-9A-HJKMNP-TV-Z]{16}$/i.test(command))throw Error('Invalid assignment command');await load();setRows(current=>current.map(r=>r.captureId===row.captureId?{...r,command,codeIssued:true}:r));setMessage('Code saved on this PC. Paste it into the addon before entering the next BG.');}catch(e){setMessage(String(e));}finally{setBusy(false);}}
 async function sync(){if(syncing.current)return;syncing.current=true;setBusy(true);try{const result=await invoke<{checked:number;acknowledged:number;waiting:number;message:string}>('sync_captures');if(!result||![result.checked,result.acknowledged,result.waiting].every(n=>Number.isSafeInteger(n)&&n>=0&&n<=3)||typeof result.message!=='string')throw Error('Invalid synchronization response');if(mounted.current&&result.checked>0)setMessage(`Last sync: ${result.acknowledged} receipts saved in this pass; ${result.waiting} checked captures still need attention. Previously saved receipts are not counted here. See Local capture queue for current totals. ${result.message}`);}catch(e){if(mounted.current)setMessage(String(e));}finally{syncing.current=false;if(mounted.current)setBusy(false);}}
  const preferenceFeedback = uploadPreference.error !== null && <p className="status-message" role="alert">{uploadPreference.error === 'read' ? 'Cannot read your saved upload preference. Automatic uploads are paused until you choose to resume.' : 'Could not save your upload preference. It applies until you close the companion.'}</p>;
 return <section className="capture-assignments"><div className="section-toolbar"><div><h2>Test assignments</h2><p>Assigned captures go to review. No rating is applied.</p></div><div className="button-group"><button disabled={busy} onClick={()=>void load()}>Refresh assignments</button><button className="button-primary" disabled={busy} onClick={()=>void sync()}>Sync saved captures</button></div></div>
 <label className="capture-auto-sync"><span><strong>Automatic uploads</strong><span>Upload saved data for your assigned matches. This preference is remembered on this PC.</span></span><input type="checkbox" checked={autoSync} onChange={e=>changeAutoSync(e.target.checked)} aria-label="Automatically upload data for assigned matches" /></label><p className="status-message" role="status">{message}</p>
    {!autoSync && <div className="uploads-paused" role="status"><span>Automatic uploads paused</span><button onClick={() => changeAutoSync(true)}>Resume uploads</button></div>}
    {preferenceFeedback}
 {rows.map(row=><article className="history-match" key={row.captureId}><h3>{row.character.name} · Darkspear</h3><p>Arm before {new Date(row.armUntil).toLocaleString()}</p>
 <button disabled={busy||row.cancelled||Date.parse(row.armUntil)<=Date.now()||(row.codeIssued&&!row.command)} onClick={()=>void issue(row)}>{row.command?'Show saved command':'Issue my one-time code'}</button>
 {row.command&&!row.cancelled&&<><label htmlFor={`command-${row.captureId}`}>Match command to paste in WoW chat</label><div className="command-field">
          <input id={`command-${row.captureId}`} readOnly value="/rbgs match ••••••••••••••••••••" onFocus={event => event.target.select()} onCopy={event => { event.preventDefault(); if (!mounted.current || Date.parse(row.armUntil) <= Date.now()) return; event.clipboardData.setData('text/plain', row.command!); setCopied(row.captureId); setCopyFallback(null); }} />
          <button className="button-primary command-copy" disabled={Date.parse(row.armUntil) <= Date.now()} onClick={() => void copy(row)}><Icon name={copied === row.captureId ? 'check' : 'copy'} />{copied === row.captureId ? 'Copied' : 'Copy command'}</button>
          <span className="visually-hidden" role="status">{copied === row.captureId ? 'Command copied.' : ''}</span>
 </div>{copyFallback === row.captureId && <p className="status-message" role="status">Press Ctrl+C to copy the selected command.</p>}</>}
 {row.codeIssued&&!row.command&&<p>The code was already issued elsewhere or its response was lost. Recover from the addon if armed; otherwise request a fresh assignment.</p>}
 </article>)}</section>;
}
