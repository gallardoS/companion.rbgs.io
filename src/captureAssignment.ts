export type Assignment = { captureId: string; assignmentId: string; expectedMapProfile?: string; character: { name: string; realm: string | null }; armUntil: string; codeIssued: boolean; cancelled: boolean; command: string | null };

export function isAssignmentCommand(value: unknown): value is string {
  return typeof value === 'string' && /^\/rbgs (?:match|test-wsg|test-forever-wsg) RB2:[0-9a-f-]{36}:[0-9A-HJKMNP-TV-Z]{16}$/i.test(value);
}

export function isAssignmentList(value: unknown): value is Assignment[] {
  return Array.isArray(value) && value.length <= 50 && value.every(row => row && typeof row.captureId === 'string' && /^[0-9a-f-]{36}$/i.test(row.captureId) && row.character && typeof row.character.name === 'string' && (row.character.realm === null || typeof row.character.realm === 'string') && typeof row.codeIssued === 'boolean' && typeof row.cancelled === 'boolean' && Number.isFinite(Date.parse(row.armUntil)) && (row.command === null || isAssignmentCommand(row.command)));
}
