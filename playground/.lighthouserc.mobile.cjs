const { chromium } = require('playwright');
const port = process.env.PLAYGROUND_PORT || '5173';
const base = `http://127.0.0.1:${port}/aozora/playground/`;

module.exports = {
  ci: {
    collect: {
      chromePath: chromium.executablePath(),
      numberOfRuns: 3,
      startServerCommand: 'bun run preview:production',
      startServerReadyPattern: `http://127.0.0.1:${port}`,
      url: [base, `${base}gallery.html`],
      settings: {
        chromeFlags: '--headless --no-sandbox --disable-dev-shm-usage',
      },
    },
    assert: {
      aggregationMethod: 'median',
      assertions: {
        'categories:accessibility': ['error', { minScore: 1 }],
        'categories:best-practices': ['error', { minScore: 1 }],
        'categories:performance': ['error', { minScore: 0.97 }],
        'categories:seo': ['error', { minScore: 1 }],
        'cumulative-layout-shift': ['error', { maxNumericValue: 0.05 }],
        'largest-contentful-paint': ['error', { maxNumericValue: 2400 }],
        'total-blocking-time': ['error', { maxNumericValue: 200 }],
      },
    },
    upload: {
      outputDir: '.lighthouseci/reports/mobile',
      target: 'filesystem',
    },
  },
};
