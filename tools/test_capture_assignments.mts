import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isAssignmentCommand, isAssignmentList } from '../src/captureAssignment.ts';

const captureId = '00000000-0000-4000-8000-000000000001';
const command = `/rbgs test-forever-wsg RB2:${captureId}:0123456789ABCDEF`;
const pending = {
  captureId, assignmentId: '00000000-0000-4000-8000-000000000002',
  expectedMapProfile: 'FOREVER_WSG_TEST', character: { name: 'Synthetic Player', realm: null },
  armUntil: '2030-01-01T00:00:00Z', codeIssued: false, cancelled: false, command: null,
};

test('Forever commands remain valid when issued and when loaded again', () => {
  assert.ok(isAssignmentCommand(command));
  assert.ok(isAssignmentList([pending]));
  assert.ok(isAssignmentList([{ ...pending, codeIssued: true, command }]));
});

test('a saved command on a cancelled Forever test does not block a new pending test', () => {
  assert.ok(isAssignmentList([{ ...pending, codeIssued: true, cancelled: true, command }, pending]));
});

test('historical Darkspear and TBC commands remain valid', () => {
  for (const action of ['match', 'test-wsg']) {
    assert.ok(isAssignmentCommand(command.replace('test-forever-wsg', action)));
  }
});

test('unknown actions, malformed codes and malformed lists are still rejected', () => {
  for (const invalid of [null, command.replace('test-forever-wsg', 'unknown'), command.slice(0, -1), `${command}\n/reload`]) {
    assert.equal(isAssignmentCommand(invalid), false);
    assert.equal(isAssignmentList([{ ...pending, command: invalid === null ? 42 : invalid }]), false);
  }
  for (const invalid of [{}, Array(51).fill(pending), [{ ...pending, armUntil: 'bad' }], [{ ...pending, character: null }]]) {
    assert.equal(isAssignmentList(invalid), false);
  }
});
