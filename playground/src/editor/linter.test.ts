import { describe, expect, it } from 'vitest';

import { diagnosticSeverity } from './linter';

describe('diagnosticSeverity', () => {
  it('preserves every wire severity at the editor boundary', () => {
    expect(diagnosticSeverity('error')).toBe('error');
    expect(diagnosticSeverity('warning')).toBe('warning');
    expect(diagnosticSeverity('note')).toBe('info');
  });
});
