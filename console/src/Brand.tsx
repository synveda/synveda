const lightLogo = new URL(
  "../../assets/brand/synveda-lockup.svg",
  import.meta.url,
).href;
const darkLogo = new URL(
  "../../assets/brand/synveda-lockup-dark.svg",
  import.meta.url,
).href;

/** The same outlined artwork as the public site, bundled by Vite. */
export function Brand() {
  return (
    <picture className="brand-artwork">
      <source media="(prefers-color-scheme: dark)" srcSet={darkLogo} />
      <img src={lightLogo} width="157" height="36" alt="Synveda" />
    </picture>
  );
}
