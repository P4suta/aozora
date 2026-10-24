const base = require('./.lighthouserc.mobile.cjs');

module.exports = {
  ...base,
  ci: {
    ...base.ci,
    assert: {
      ...base.ci.assert,
      assertions: {
        ...base.ci.assert.assertions,
        'categories:performance': ['error', { minScore: 0.97 }],
        'largest-contentful-paint': ['error', { maxNumericValue: 2400 }],
        'total-blocking-time': ['error', { maxNumericValue: 200 }],
      },
    },
  },
};
