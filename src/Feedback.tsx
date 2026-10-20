import type { ReactNode } from 'react';

export function errorDetail(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error && typeof error.message === 'string') return error.message;
  return String(error);
}

export function Feedback({ title, children, detail, action }: { title: string; children: ReactNode; detail?: string | null; action?: ReactNode }) {
  return <div className="feedback" role="alert">
    <div className="feedback-heading"><strong>{title}</strong>{action}</div>
    <p>{children}</p>
    {detail && <details><summary>Error details</summary><p>{detail}</p></details>}
  </div>;
}
