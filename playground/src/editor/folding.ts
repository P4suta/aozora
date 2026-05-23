import { foldService } from '@codemirror/language';
import { parserStateField } from './parserState';

/**
 * Fold containerOpen/containerClose blocks ([＃ここから2字下げ］...
 * ［＃ここで字下げ終わり］). The pre-computed `containerFolds`
 * array on `parserStateField` carries (openLineEnd, closeStart) for
 * every detected pair.
 */
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
