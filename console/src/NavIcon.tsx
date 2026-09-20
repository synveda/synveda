import type { RouteId } from "./routes.mjs";

const paths: Partial<Record<RouteId, string>> = {
  home: "m3 10 9-7 9 7 M5 9v12h5v-7h4v7h5V9",
  sessions:
    "M8 3H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h3 M8 3h11a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-7l-4 3V3 M12 8h5 M12 12h5",
  knowledge: "M12 5c-3-2-6-2-9-1v15c3-1 6-1 9 1 3-2 6-2 9-1V4c-3-1-6-1-9 1v15",
  learnings: "M5 4h14v16H5z M8 8h8 M8 12h8 M8 16h4",
  context: "m12 3 9 5-9 5-9-5 9-5 M3 12l9 5 9-5 M3 16l9 5 9-5",
  skills: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  operations: "M3 12h4l3-8 4 16 3-8h4",
  okf: "M8 3v14m-4-4 4 4 4-4 M16 21V7m-4 4 4-4 4 4",
  tools: "M9 3v5 M15 3v5 M6 8h12v3a6 6 0 0 1-12 0V8 M12 17v4",
  people:
    "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2 M9 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M17 4a4 4 0 0 1 0 7 M22 21v-2a4 4 0 0 0-3-4",
  settings: "M4 6h16 M4 12h16 M4 18h16 M8 3v6 M16 9v6 M10 15v6",
  reviews: "M4 4h16v16H4z m4 8 3 3 5-6",
  scopes: "M9 3h6v5H9z M3 16h6v5H3z M15 16h6v5h-6z M12 8v4H6v4 M12 12h6v4",
  configuration: "M4 6h16 M4 12h16 M4 18h16 M8 3v6 M16 9v6 M10 15v6",
  audit: "M12 2 3 6v6c0 5 9 10 9 10s9-5 9-10V6l-9-4 m-4 10 3 3 5-6",
  "service-identities":
    "M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M4 21v-2a5 5 0 0 1 5-5h6a5 5 0 0 1 5 5v2",
  welcome: "M12 4v16 M4 12h16",
};

export function NavIcon({ route }: { route: RouteId }) {
  return (
    <svg
      className="nav-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={paths[route] ?? paths.knowledge} />
    </svg>
  );
}
