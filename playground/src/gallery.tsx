// biome-ignore-all lint/security/noDangerouslySetInnerHtml: Renderer-owned HTML is mounted only at the gallery preview trust boundary.
import '@react-spectrum/s2/page.css';

import { loadPreferences } from '@aozora/playground-ui/storage';
import { Button } from '@react-spectrum/s2/Button';
import { Content, Heading } from '@react-spectrum/s2/Dialog';
import { InlineAlert } from '@react-spectrum/s2/InlineAlert';
import { Link } from '@react-spectrum/s2/Link';
import { ProgressCircle } from '@react-spectrum/s2/ProgressCircle';
import { Provider } from '@react-spectrum/s2/Provider';
import { style } from '@react-spectrum/s2/style' with { type: 'macro' };
import { StrictMode, useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import '../../crates/aozora/assets/aozora-notation.css';
import { GALLERY_CATALOG } from './gallery-catalog';
import type { GalleryPanel } from './gallery-engine';
import './gallery.css';
import './styles/renderer-theme.css';

const GALLERY_LOAD_DELAY_MS = 100;

const pageStyle = style({
  color: 'neutral',
  display: 'flex',
  flexDirection: 'column',
  marginX: 'auto',
  maxWidth: 1200,
  padding: 32,
  rowGap: 24,
});

const headerStyle = style({
  display: 'flex',
  flexDirection: 'column',
  rowGap: 8,
});

const titleStyle = style({
  font: 'heading-xl',
  margin: 0,
});

const descriptionStyle = style({
  color: 'gray-700',
  font: 'body-lg',
  margin: 0,
});

const loadingStyle = style({
  alignItems: 'center',
  display: 'flex',
  flexDirection: 'column',
  justifyContent: 'center',
  minHeight: 240,
  rowGap: 12,
});

const sectionStyle = style({
  display: 'flex',
  flexDirection: 'column',
  rowGap: 12,
});

const sectionTitleStyle = style({
  font: 'heading-lg',
  margin: 0,
});

function useSystemDark(): boolean {
  const [dark, setDark] = useState(
    () =>
      globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false,
  );
  useEffect(() => {
    const media = globalThis.matchMedia?.('(prefers-color-scheme: dark)');
    if (!media) return;
    const update = () => setDark(media.matches);
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);
  return dark;
}

function GalleryApp() {
  const [preferences] = useState(loadPreferences);
  const [panels, setPanels] = useState<readonly GalleryPanel[] | null>(null);
  const [failed, setFailed] = useState(false);
  const systemDark = useSystemDark();
  const locale = preferences.locale;
  const text = GALLERY_CATALOG[locale];
  const colorScheme =
    preferences.colorScheme === 'auto'
      ? systemDark
        ? 'dark'
        : 'light'
      : preferences.colorScheme;

  const load = useCallback(() => {
    setFailed(false);
    setPanels(null);
    void import('./gallery-engine')
      .then(({ renderGallery }) => renderGallery())
      .then(setPanels, () => setFailed(true));
  }, []);

  useEffect(() => {
    document.documentElement.lang = locale;
    document.documentElement.dataset.colorScheme = colorScheme;
    document.title = text.title;
  }, [colorScheme, locale, text.title]);

  useEffect(() => {
    let secondFrame = 0;
    let timer = 0;
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => {
        timer = globalThis.setTimeout(load, GALLERY_LOAD_DELAY_MS);
      });
    });
    return () => {
      cancelAnimationFrame(firstFrame);
      cancelAnimationFrame(secondFrame);
      globalThis.clearTimeout(timer);
    };
  }, [load]);

  return (
    <Provider
      background="base"
      colorScheme={colorScheme}
      locale={locale === 'ja' ? 'ja-JP' : 'en-US'}
    >
      <main className={pageStyle}>
        <header className={headerStyle}>
          <h1 className={titleStyle}>{text.title}</h1>
          <p className={descriptionStyle}>{text.description}</p>
          <Link href="./">{text.back}</Link>
        </header>

        {failed && (
          <div className={loadingStyle}>
            <InlineAlert variant="negative">
              <Heading>{text.failure}</Heading>
              <Content>{text.retryHint}</Content>
            </InlineAlert>
            <Button onPress={load} variant="accent">
              {text.retry}
            </Button>
          </div>
        )}

        {!failed && panels === null && (
          <div className={loadingStyle} role="status">
            <ProgressCircle aria-label={text.loadingLabel} isIndeterminate />
            <span>{text.loading}</span>
          </div>
        )}

        {panels?.map((panel) => (
          <section
            className={sectionStyle}
            data-family={panel.family}
            key={panel.family}
          >
            <h2 className={sectionTitleStyle}>{panel.label[locale]}</h2>
            <div className="gallery-columns">
              <section
                aria-label={`${panel.label[locale]} — ${text.horizontal}`}
                className="gallery-h"
              >
                <span className="gallery-mode">{text.horizontal}</span>
                <div
                  className="gallery-preview html-preview aozora-notation"
                  dangerouslySetInnerHTML={{ __html: panel.html }}
                />
              </section>
              <section
                aria-label={`${panel.label[locale]} — ${text.vertical}`}
                className="gallery-v"
              >
                <span className="gallery-mode">{text.vertical}</span>
                <div
                  className="gallery-preview html-preview aozora-notation aozora-vertical"
                  dangerouslySetInnerHTML={{ __html: panel.html }}
                />
              </section>
            </div>
          </section>
        ))}
      </main>
    </Provider>
  );
}

const root = document.getElementById('root');
if (root === null) {
  throw new Error('#root missing from gallery.html');
}

createRoot(root).render(
  <StrictMode>
    <GalleryApp />
  </StrictMode>,
);
