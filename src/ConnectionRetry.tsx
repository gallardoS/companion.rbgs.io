import { useId, useState } from 'react';
import { Icon } from './Navigation';

export function ConnectionRetry({ checking, disabled, onRetry }: { checking: boolean; disabled: boolean; onRetry: () => void }) {
  const tooltipId = useId();
  const [dismissed, setDismissed] = useState(false);
  return <div className="connection-retry" data-tooltip-dismissed={dismissed}
    onPointerEnter={() => setDismissed(false)}
    onKeyDown={event => { if (event.key === 'Escape') setDismissed(true); }}>
    <button type="button" className="connection-retry-button" aria-label="Check connection" aria-describedby={tooltipId}
      aria-busy={checking} disabled={disabled || checking} onFocus={() => setDismissed(false)} onClick={onRetry}>
      <Icon name="reload" />
    </button>
    <span id={tooltipId} className="connection-retry-tooltip" role="tooltip">Check connection</span>
  </div>;
}
