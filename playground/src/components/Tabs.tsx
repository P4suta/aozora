import { createSignal, For, JSX } from 'solid-js';
import { loadNumber, saveNumber } from '../storage';

export interface TabSpec {
  label: string;
  render: () => JSX.Element;
}

interface TabsProps {
  tabs: TabSpec[];
  /**
   * Optional localStorage namespace. When provided, the active tab
   * index is persisted under `tabs:${storageKey}` so a page reload
   * lands on the same tab the user last selected.
   */
  storageKey?: string;
}

function readInitialIndex(props: TabsProps): number {
  if (!props.storageKey) return 0;
  const saved = loadNumber(`tabs:${props.storageKey}`);
  if (saved === null || saved < 0 || saved >= props.tabs.length) return 0;
  return saved;
}

export default function Tabs(props: TabsProps) {
  const [active, setActive] = createSignal(readInitialIndex(props));

  function selectTab(i: number) {
    setActive(i);
    if (props.storageKey) saveNumber(`tabs:${props.storageKey}`, i);
  }

  return (
    <div class="tabs">
      <div role="tablist" class="tab-bar">
        <For each={props.tabs}>
          {(tab, i) => (
            <button
              type="button"
              role="tab"
              aria-selected={active() === i()}
              class={active() === i() ? 'tab active' : 'tab'}
              onClick={() => selectTab(i())}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>
      <div class="tab-panel" role="tabpanel">
        {props.tabs[active()]!.render()}
      </div>
    </div>
  );
}
