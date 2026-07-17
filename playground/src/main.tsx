/* @refresh reload */
import { render } from 'solid-js/web';
import App from './App';
import { bootstrapTheme } from './theme';
import { bootstrapLang } from './i18n';
import './styles.css';
// レンダラ所有の正準記法スタイルシート（単一の権威）。テーマ橋渡しと枠の
// レイアウトは続く aozora.css が上書きする。
import '../../crates/aozora/assets/aozora-notation.css';
import './aozora.css';

bootstrapTheme();
bootstrapLang();

const root = document.getElementById('root');
if (!root) {
  throw new Error('Missing #root element');
}

render(() => <App />, root);
