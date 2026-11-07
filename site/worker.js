// zapinator.kunalbagaria.com: the one-line installer and the release downloads.
//
//   /                    the install script, as plain text (what `curl … | bash` runs,
//   /install.sh          and what a browser shows, so you can read it first)
//   /latest              "<version> <sha256>" of the current release
//   /releases/*.tar.gz   the app bundles
//
// The files themselves are static assets that scripts/release.sh puts in
// dist/site; this only picks the path and the headers.

const SCRIPT_PATHS = new Set(["/", "/install", "/install.sh"]);
const PLAIN_TEXT = new Set(["/install.sh", "/latest"]);

export default {
  async fetch(request, env) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("method not allowed\n", { status: 405, headers: { allow: "GET, HEAD" } });
    }

    const url = new URL(request.url);
    const path = SCRIPT_PATHS.has(url.pathname) ? "/install.sh" : url.pathname;
    const res = await env.ASSETS.fetch(new Request(new URL(path, url), request));

    if (res.status === 404) {
      return new Response("not found\n", { status: 404, headers: { "content-type": "text/plain; charset=utf-8" } });
    }

    const headers = new Headers(res.headers);
    headers.set("x-content-type-options", "nosniff");
    if (PLAIN_TEXT.has(path)) {
      headers.set("content-type", "text/plain; charset=utf-8");
      headers.set("cache-control", "no-cache");
    }
    return new Response(res.body, { status: res.status, headers });
  },
};
