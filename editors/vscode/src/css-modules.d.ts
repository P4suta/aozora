// esbuild's `text` loader turns a `.css` import into its contents as a
// string (see esbuild.mjs). Declare that shape for `tsc --noEmit`.
declare module "*.css" {
  const content: string;
  export default content;
}
