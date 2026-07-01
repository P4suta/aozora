/* @refresh reload */
import { render } from 'solid-js/web';
import App from './App';
import { bootstrapTheme } from './theme';
import { bootstrapLang } from './i18n';
import './styles.css';
import './aozora.css';

bootstrapTheme();
bootstrapLang();

const root = document.getElementById('root');
if (!root) {
  throw new Error('Missing #root element');
}

render(() => <App />, root);
