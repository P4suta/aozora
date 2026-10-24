const base = require('./.lighthouserc.cjs');

module.exports = {
  ...base,
  ci: {
    ...base.ci,
    assert: {
      ...base.ci.assert,
      assertions: {
        ...base.ci.assert.assertions,
        'categories:performance': ['error', { minScore: 0.99 }],
        'largest-contentful-paint': ['error', { maxNumericValue: 1500 }],
        'total-blocking-time': ['error', { maxNumericValue: 150 }],
      },
    },
  },
};
