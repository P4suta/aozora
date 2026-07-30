import { PlaygroundApp } from '@aozora/playground-ui';

import { aozoraPlaygroundAdapter } from './adapter';

export default function App() {
  return <PlaygroundApp adapter={aozoraPlaygroundAdapter} />;
}
