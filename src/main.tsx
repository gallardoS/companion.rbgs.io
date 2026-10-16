import { StrictMode, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { createRoot } from 'react-dom/client';
import { t } from './locales/en';
import { SavedVariables } from './SavedVariables';
import { CaptureAssignments } from './CaptureAssignments';
import './styles.css';

function App() {
  const [pairing, setPairing] = useState<{ code: string; expiresAt: string; approvalUrl: string } | null>(null);
  const [connected, setConnected] = useState(false);
  const [capturePermissions, setCapturePermissions] = useState(false);
  const [accountId, setAccountId] = useState('');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('Checking saved connection…');
  const inFlight = useRef(false);
  const checkEpoch = useRef(0);
  async function check() {
    const epoch=++checkEpoch.current;
    try { const status = await invoke<{ accountId: string; localStorageWarning?: string | null; capturePermissions: boolean }>('connection_status'); if(epoch!==checkEpoch.current)return; setConnected(true); setAccountId(status.accountId ?? ''); setCapturePermissions(status.capturePermissions === true); setMessage(status.localStorageWarning ? `Connected to your rbgs.io account. ${status.localStorageWarning}` : 'Connected to your rbgs.io account.'); }
    catch (error) { if(epoch!==checkEpoch.current)return; setConnected(false); setAccountId(''); setCapturePermissions(false); setMessage(String(error)); }
  }
  useEffect(() => { const refresh=()=>{if(!pairing)void check();};refresh();const timer=window.setInterval(refresh,15000);window.addEventListener('focus',refresh);return()=>{++checkEpoch.current;window.clearInterval(timer);window.removeEventListener('focus',refresh);}; }, [pairing]);
  useEffect(() => {
    if (!pairing) return;
    let disposed = false;
    const poll = async () => {
      if (inFlight.current) return;
      inFlight.current = true;
      try {
        const status = await invoke<string>('poll_pairing');
        if (disposed) return;
        if (status === 'PAIRED') { setPairing(null); await check(); }
      } catch (error) { if (!disposed) setMessage(String(error)); }
      finally { inFlight.current = false; }
    };
    const timer = window.setInterval(() => void poll(), 5000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [pairing]);
  async function start(captures = true) {
    if (busy) return;
    setBusy(true);
    try {
      const result = await invoke<{ code: string; expiresAt: string; approvalUrl: string }>('start_pairing', { capturePermissions: captures });
      setPairing(result); setMessage('Waiting for your approval in web Settings.');
      try { await invoke('open_pairing_browser'); } catch { setMessage('Connection request created. Use Approve in browser, or the manual code option if your browser cannot open.'); }
    } catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }
  return <main><h1>{t('title')}</h1><p role="status">{message}</p>
    {!connected && <>
      <button disabled={busy} onClick={() => void start()}>{pairing ? 'Generate a new code' : 'Connect account'}</button></>}
    {pairing && <section aria-label="Pairing"><p>Approve this PC and its requested permissions in your browser. Connection finishes automatically.</p>
      <button disabled={busy} onClick={()=>void invoke('open_pairing_browser').catch(error=>setMessage(String(error)))}>Approve in browser</button>
      <details><summary>Use a code manually</summary><output className="pairing-code">{pairing.code}</output></details><p>Expires at {new Date(pairing.expiresAt).toLocaleTimeString()}.</p>
      <button disabled={busy} onClick={() => void invoke<string>('poll_pairing').then(status => { if (status === 'PAIRED') { setPairing(null); void check(); } else { setMessage('Waiting for your approval in web Settings.'); } }).catch(error =>setMessage(String(error)))}>Check approval</button></section>}
    <button disabled={busy} onClick={() => void check()}>Check connection</button>
    {connected && !capturePermissions && <button disabled={busy} onClick={() => void start(true)}>Enable capture permissions</button>}
    {pairing && <p>If enabling captures, approve assignment access and saved-capture uploads in web Settings.</p>}
    <p>{connected ? 'Manage or revoke this device in web Settings.' : 'Approve only the code shown in your own companion.'}</p>
    <SavedVariables />
    {connected && capturePermissions && <CaptureAssignments key={accountId} />}
  </main>;
}

createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);
