import { createHash, createHmac } from "node:crypto";
import { execFileSync } from "node:child_process";
const [db] = process.argv.slice(2);
const KEY = process.env.S159_KEY;
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const hash = (t) => b64u(createHash("sha256").update(t).digest());
const sign = (t) => b64u(createHmac("sha256", KEY).update(t).digest());
const T0 = Date.parse("2026-11-21T09:00:00Z"); // fixed: relative times must not drift between the two seeds
const real = Date.now();
const tok = "s159-admin-token-0001";
let sql = `
INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('user-admin',1001,'ada-admin','admin','',${T0},${T0},${T0});
INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES ('s-admin','${hash(tok)}','user-admin',${real + 36e5 * 8},${T0},${T0});
INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (42,42,'acme-labs','Organization','Organization','all',${T0},${T0});
INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('user-admin',42,'admin',${T0});
`;
const repos = ["web-app", "api-service", "ios-client", "docs", "infra", "tools", "billing", "search", "mailer"];
repos.forEach((n, i) => { sql += `INSERT INTO repositories (id,installation_id,owner,name,full_name,private,archived,default_branch,html_url,permission,last_synced_at,created_at,updated_at) VALUES (${1101 + i},42,'acme-labs','${n}','acme-labs/${n}',0,0,'main','https://github.com/acme-labs/${n}','admin',${T0},${T0},${T0});\n`; });
sql += `INSERT INTO bitbucket_connections (id,name,workspace,workspace_uuid,access_token_key,created_by,created_at,updated_at) VALUES ('bb-1','Acme Bitbucket','acme-ws','{11111111-1111-1111-1111-111111111111}','s159-key-bb-1','user-admin',${T0},${T0});\n`;
sql += `INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_by,created_at,updated_at) VALUES
 ('gl-1','Acme / web','https://gitlab.com','project',11,'acme/web','s159-gl-tok-1','s159-gl-hook-1','user-admin',${T0},${T0}),
 ('gl-2','Acme / mobile','https://gitlab.com','project',12,'acme/mobile','s159-gl-tok-2','s159-gl-hook-2','user-admin',${T0},${T0});\n`;
const pool = (id, name, scope, repoId, i) => `INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,desired_count,min_count,max_count,provider,providers,created_by,created_at,updated_at) VALUES ('${id}',42,${repoId ?? "NULL"},'${name}','${scope}','["linux"]','ghcr.io/actions/actions-runner:latest',0,0,4,'docker','["docker"]','user-admin',${T0 - i * 60000},${T0 - i * 60000});\n`;
const pools = [
  ["pool-gh-one", "gh-one", "repository", 1101], ["pool-gh-many", "gh-many", "repository", 1101], ["pool-gh-nine", "gh-nine", "repository", 1101],
  ["pool-gh-org", "gh-org", "organization", null], ["pool-bb", "bb-pool", "repository", 1103],
  ["pool-gl-one", "gl-one", "gitlab", null], ["pool-gl-two", "gl-two", "gitlab", null], ["pool-gl-zero", "gl-zero", "gitlab", null],
  ["pool-unknown", "unknown-scope", "azure-devops", null],
];
pools.forEach((p, i) => { sql += pool(...p, i); });
const memb = (p, rs) => rs.forEach((r) => { sql += `INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('${p}',${r},${T0});\n`; });
memb("pool-gh-one", [1101]); memb("pool-gh-many", [1101, 1102, 1103]); memb("pool-gh-nine", [1101, 1102, 1103, 1104, 1105, 1106, 1107, 1108, 1109]); memb("pool-bb", [1103]);
sql += `INSERT INTO runner_pool_bitbucket_connections (pool_id,connection_id,created_at) VALUES ('pool-bb','bb-1',${T0});\n`;
sql += `INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('pool-gl-one','gl-1',${T0}),('pool-gl-two','gl-1',${T0}),('pool-gl-two','gl-2',${T0 + 1000});\n`;
execFileSync("sqlite3", [db], { input: sql, stdio: ["pipe", "inherit", "inherit"] });
console.log("seeded", db, "cookie", `${tok}.${sign(tok)}`.length);
