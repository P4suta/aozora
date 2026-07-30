import { GALLERY_FIXTURES } from './gallery-fixtures';
import { Document, ensureWasmReady } from './wasm-loader';

export interface GalleryPanel {
  readonly family: string;
  readonly label: (typeof GALLERY_FIXTURES)[number]['label'];
  readonly html: string;
}

export async function renderGallery(): Promise<readonly GalleryPanel[]> {
  await ensureWasmReady();
  return GALLERY_FIXTURES.map((fixture) => {
    const document = new Document(fixture.source);
    try {
      return {
        family: fixture.family,
        label: fixture.label,
        html: document.toHtml(),
      };
    } finally {
      document.free();
    }
  });
}
