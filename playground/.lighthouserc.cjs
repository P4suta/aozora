const { chromium } = require('playwright');

module.exports = {
  ci: {
    collect: {
      chromePath: chromium.executablePath(),
      numberOfRuns: 2,
      startServerCommand: 'bun run preview:lighthouse',
      startServerReadyPattern: 'http://127.0.0.1:5173',
      url: [
        'http://127.0.0.1:5173/aozora/playground/',
        'http://127.0.0.1:5173/aozora/playground/gallery.html',
      ],
      settings: {
        chromeFlags: '--headless --no-sandbox --disable-dev-shm-usage',
        preset: 'desktop',
      },
    },
    assert: {
      assertions: {
        'categories:accessibility': ['error', { minScore: 1 }],
        'categories:best-practices': ['error', { minScore: 1 }],
        'categories:performance': ['error', { minScore: 0.95 }],
        'categories:seo': ['error', { minScore: 1 }],
        'cumulative-layout-shift': ['error', { maxNumericValue: 0.1 }],
        'largest-contentful-paint': ['error', { maxNumericValue: 2500 }],
        'total-blocking-time': ['error', { maxNumericValue: 300 }],
      },
    },
    upload: {
      outputDir: '.lighthouseci/reports/desktop',
      target: 'filesystem',
    },
  },
};
