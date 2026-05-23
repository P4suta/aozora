interface CodeViewProps {
  code: string;
  lang?: string;
}

function tryPretty(code: string, lang?: string): string {
  if (lang !== 'json') return code;
  try {
    return JSON.stringify(JSON.parse(code), null, 2);
  } catch {
    return code;
  }
}

export default function CodeView(props: CodeViewProps) {
  return (
    <pre class="code-view">
      <code data-lang={props.lang ?? 'text'}>{tryPretty(props.code, props.lang)}</code>
    </pre>
  );
}
