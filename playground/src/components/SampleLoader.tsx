import { For } from 'solid-js';
import { SAMPLES } from '../samples';

interface SampleLoaderProps {
  /** Called with the selected sample's body text and its display title. */
  onPick: (text: string, title: string) => void;
}

export default function SampleLoader(props: SampleLoaderProps) {
  function onChange(event: Event & { currentTarget: HTMLSelectElement }) {
    const id = event.currentTarget.value;
    const sample = SAMPLES.find((s) => s.id === id);
    if (sample) props.onPick(sample.text, sample.title);
    event.currentTarget.value = '';
  }

  return (
    <label class="sample-loader">
      <span class="sample-label">サンプル:</span>
      <select onChange={onChange} value="">
        <option value="" disabled>
          選択してください…
        </option>
        <For each={SAMPLES}>
          {(sample) => <option value={sample.id}>{sample.title}</option>}
        </For>
      </select>
    </label>
  );
}
