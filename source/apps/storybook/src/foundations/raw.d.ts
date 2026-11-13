/// <reference types="vite/client" />

// Vite turns `?raw` imports into the file's text; the foundation pages read the product CSS that way.
declare module "*.css?raw" {
  const content: string;
  export default content;
}
