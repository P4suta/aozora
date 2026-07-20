import { EditorView } from '@codemirror/view';
import Tabs, { type TabSpec } from './Tabs';
import HtmlPreview from './HtmlPreview';
import CodeView from './CodeView';
import DiagnosticsView from './DiagnosticsView';
import OutlinePanel from './OutlinePanel';
import type { DiagnosticEntry, HeadingEntry } from '../editor';

interface PreviewPaneProps {
  html: string;
  serialized: string;
  diagnostics: DiagnosticEntry[];
  nodesJson: string;
  headings: HeadingEntry[];
  view: EditorView | null;
}

export default function PreviewPane(props: PreviewPaneProps) {
  const tabs: TabSpec[] = [
    { label: 'HTML preview', render: () => <HtmlPreview html={props.html} /> },
    {
      label: 'Outline',
      render: () => <OutlinePanel headings={props.headings} view={props.view} />,
    },
    { label: 'HTML source', render: () => <CodeView code={props.html} lang="html" /> },
    { label: 'Serialize', render: () => <CodeView code={props.serialized} lang="text" /> },
    {
      label: 'Diagnostics',
      render: () => <DiagnosticsView diagnostics={props.diagnostics} />,
    },
    { label: 'Nodes (JSON)', render: () => <CodeView code={props.nodesJson} lang="json" /> },
  ];
  return <Tabs tabs={tabs} storageKey="preview" />;
}
