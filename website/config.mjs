export const repository = "https://github.com/synveda/synveda";
// Pages API verified 2026-09-20: workflow build, project base, no CNAME.
export const defaultSiteUrl = "https://synveda.github.io/synveda/";
export function siteUrl() {
  const url = new URL(process.env.SITE_URL || defaultSiteUrl);
  if (
    url.protocol !== "https:" ||
    url.search ||
    url.hash ||
    url.username ||
    url.password
  ) {
    throw new Error(
      "SITE_URL must be an absolute HTTPS URL without query, fragment or credentials",
    );
  }
  if (!url.pathname.endsWith("/")) url.pathname += "/";
  return url;
}
// Public build allowlist. Reference boards and repository/runtime trees never enter dist.
export const publicFiles = [
  ["website/index.html", "index.html"],
  ["website/styles.css", "styles.css"],
  ["assets/product/review-learning.png", "assets/review-learning.png"],
  ["assets/brand/synveda-lockup.svg", "assets/synveda-lockup.svg"],
  ["assets/brand/synveda-lockup-dark.svg", "assets/synveda-lockup-dark.svg"],
  ["assets/brand/synveda-mark.svg", "assets/synveda-mark.svg"],
  ["assets/brand/favicon.svg", "assets/favicon.svg"],
  ["assets/brand/favicon-32.png", "assets/favicon-32.png"],
  ["assets/brand/social-preview.png", "assets/social-preview.png"],
  ["assets/brand/fonts/InterVariable.woff2", "assets/InterVariable.woff2"],
  ["assets/brand/fonts/OFL.txt", "assets/OFL.txt"],
  ["assets/brand/ATTRIBUTIONS.md", "assets/ATTRIBUTIONS.md"],
];
