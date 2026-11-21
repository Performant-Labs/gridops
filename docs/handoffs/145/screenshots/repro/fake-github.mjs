import http from "node:http";
const port = Number(process.argv[2]);
const repos = {
  42: [
    [1101, "acme-labs/web-app", false],
    [1102, "acme-labs/api-service", true],
    [1103, "acme-labs/ios-client", true],
  ],
  43: [[1201, "ada-admin/dotfiles", false]],
};
const send = (res, body, status = 200) => {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
};
http
  .createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    const m = url.pathname.match(/^\/app\/installations\/(\d+)\/access_tokens$/);
    if (req.method === "POST" && m)
      return send(res, { token: `tok-${m[1]}`, expires_at: new Date(Date.now() + 3600e3).toISOString() }, 201);
    if (url.pathname === "/app") return send(res, { slug: "gridops-audit", owner: { login: "acme-labs", type: "Organization" } });
    if (url.pathname === "/installation/repositories") {
      const id = (req.headers.authorization ?? "").replace(/^\S+ tok-/, "");
      const list = (repos[id] ?? []).map(([rid, full, priv]) => ({
        id: rid, name: full.split("/")[1], full_name: full, private: priv, archived: false,
        default_branch: "main", html_url: `https://github.com/${full}`, updated_at: new Date().toISOString(),
        owner: { login: full.split("/")[0] }, permissions: { admin: true },
      }));
      return send(res, { total_count: list.length, repositories: list });
    }
    if (/^\/orgs\/[^/]+\/actions\/runner-groups$/.test(url.pathname))
      return send(res, { runner_groups: [
        { id: 1, name: "Default", visibility: "all", default: true },
        { id: 7, name: "macOS builds", visibility: "selected", default: false },
      ] });
    send(res, { message: "not found " + url.pathname }, 404);
  })
  .listen(port, "127.0.0.1", () => console.log("fake github on", port));
