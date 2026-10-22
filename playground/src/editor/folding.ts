import { foldService } from '@codemirror/language';
import { parserStateField } from './parserState';

export const aozoraFolding = foldService.of((state, lineStart, lineEnd) => {
  const ps = state.field(parserStateField);
  if (ps.containerFolds.length === 0) return null;
  for (const fold of ps.containerFolds) {
    if (fold.openLineEnd >= lineStart && fold.openLineEnd <= lineEnd) {
      return { from: fold.openLineEnd, to: fold.closeStart };
    }
  }
  return null;
});
