import '@react-spectrum/s2/page.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import '../../crates/aozora/assets/aozora-notation.css';
import App from './App';
import './styles/renderer-theme.css';
import './styles/workspace.css';

const root = document.getElementById('root');
if (root === null) {
  throw new Error('#root missing from index.html');
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
