// Lets `tsc --noEmit` (this package's check-types) resolve CSS-module imports.
// Next.js transpiles this package and handles the real CSS at build time.
declare module "*.module.css" {
  const classes: { readonly [key: string]: string };
  export default classes;
}
