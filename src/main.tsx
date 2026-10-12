import { StrictMode, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { createRoot } from 'react-dom/client';
import { t } from './locales/en';
import './styles.css';

function App() {
  const [name, setName] = useState('My Windows PC');
  const [pairing, setPairing] = useState<{ code: string; expiresAt: string } | null>(null);
  const [connected, setConnected] = useState(false);
  const [capturePermissions, setCapturePermissions] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('Checking saved connectionâ€¦');
  const inFlight = useRef(false);
  const checkEpoch = useRef(0);
  async function check() {
    const epoch=++checkEpoch.current;
    try { const status = await invoke<{ accountId: string; localStorageWarning?: string | null; capturePermissions: boolean }>('connection_status'); if(epoch!==checkEpoch.current)return; setConnected(true); setCapturePermissions(status.capturePermissions === true); setMessage(status.localStorageWarning ? `Connected to your rbgs.io account. ${status.localStorageWarning}` : 'Connected to your rbgs.io account.'); }
    catch (error) { if(epoch!==checkEpoch.current)return; setConnected(false); setCapturePermissions(false); setMessage(String(error)); }
  }
  useEffect(() => { void check(); return () => { ++checkEpoch.current; }; }, []);
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
  async function start(captures = false) {
    if (busy) return;
    setBusy(true);
    try {
      const result = await invoke<{ code: string; expiresAt: string }>('start_pairing', { name, capturePermissions: captures });
      setPairing(result); setMessage('Waiting for your approval in web Settings.');
    } catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }
  return <main><h1>{t('title')}</h1><p role="status">{message}</p>
    {!connected && <><label htmlFor="device-name">Device name</label><input id="device-name" value={name} maxLength={80} disabled={busy} onChange={event => setName(event.target.value)} />
      <button disabled={busy || !name.trim()} onClick={() => void start()}>{pairing ? 'Generate a new code' : 'Connect account'}</button></>}
    {pairing && <section aria-label="Pairing"><p>Enter this code in rbgs.io web Settings to approve this PC. Connection finishes automatically.</p>
      <output className="pairing-code">{pairing.code}</output><p>Expires at {new Date(pairing.expiresAt).toLocaleTimeString()}.</p>
      <button disabled={busy} onClick={() => void invoke<string>('poll_pairing').then(status => { if (status === 'PAIRED') { setPairing(null); void check(); } else { setMessage('Waiting for your approval in web Settings.'); } }).catch(error =>setMessage(String(error)))}>Check approval</button></section>}
    <button disabled={busy} onClick={() => void check()}>Check connection</button>
    {connected && !capturePermissions && <button disabled={busy} onClick={() => void start(true)}>Enable capture permissions</button>}
    {pairing && <p>If enabling captures, approve assignment access and saved-capture uploads in web Settings.</p>}
    <p>{connected ? 'Manage or revoke this device in web Settings.' : 'Approve only the code shown in your own companion.'}</p>
  </main>;
}

createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);
