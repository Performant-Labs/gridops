import { createHash, createHmac } from "node:crypto";
import { execFileSync } from "node:child_process";
const [db, mode] = process.argv.slice(2); // mode: empty | populated
const SECRET = "audit-session-secret-for-s-walk";
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const hash = (t) => b64u(createHash("sha256").update(t).digest());
const sign = (t) => b64u(createHmac("sha256", SECRET).update(t).digest());
const now = Date.now();
const tokens = { admin: "audit-admin-token-0001", member: "audit-member-token-0001" };
let sql = `
INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES
 ('user-admin',1001,'ada-admin','admin','',${now},${now},${now}),
 ('user-member',1002,'mel-member','member','',${now},${now},${now});
INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES
 ('s-admin','${hash(tokens.admin)}','user-admin',${now + 36e5 * 8},${now},${now}),
 ('s-member','${hash(tokens.member)}','user-member',${now + 36e5 * 8},${now},${now});
`;
if (mode === "populated") {
  sql += `
INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES
 (42,42,'acme-labs','Organization','Organization','all',${now},${now}),
 (43,43,'ada-admin','User','User','selected',${now},${now});
INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES
 ('user-admin',42,'admin',${now}),('user-admin',43,'admin',${now}),
 ('user-member',42,'member',${now});
INSERT INTO repositories (id,installation_id,owner,name,full_name,private,archived,default_branch,html_url,permission,last_synced_at,created_at,updated_at) VALUES
 (1101,42,'acme-labs','web-app','acme-labs/web-app',0,0,'main','https://github.com/acme-labs/web-app','admin',${now},${now},${now}),
 (1102,42,'acme-labs','api-service','acme-labs/api-service',1,0,'main','https://github.com/acme-labs/api-service','admin',${now},${now},${now}),
 (1103,42,'acme-labs','ios-client','acme-labs/ios-client',1,0,'main','https://github.com/acme-labs/ios-client','admin',${now},${now},${now}),
 (1201,43,'ada-admin','dotfiles','ada-admin/dotfiles',0,0,'main','https://github.com/ada-admin/dotfiles','admin',${now},${now},${now});
INSERT INTO bitbucket_connections (id,name,workspace,workspace_uuid,access_token_key,created_by,created_at,updated_at) VALUES
 ('bb-1','Acme Bitbucket','acme-ws','{11111111-1111-1111-1111-111111111111}','audit-key-bb-1','user-admin',${now - 36e5},${now - 36e5}),
 ('bb-2','Platform Team','platform-ws','{22222222-2222-2222-2222-222222222222}','audit-key-bb-2','user-admin',${now - 864e5},${now - 864e5});
INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,desired_count,min_count,max_count,provider,providers,created_by,created_at,updated_at) VALUES
 ('pool-1',42,1103,'ios-builds','repository','["ios","macos"]','gridops-macos-tahoe-base',0,0,4,'tart','["tart"]','user-admin',${now},${now}),
 ('pool-2',42,NULL,'linux-org','organization','["linux"]','ghcr.io/actions/actions-runner:latest',0,0,6,'docker','["docker"]','user-admin',${now},${now});
INSERT OR IGNORE INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('pool-1',1103,${now}),('pool-1',1101,${now});
INSERT INTO runner_pool_bitbucket_connections (pool_id,connection_id,created_at) VALUES ('pool-1','bb-1',${now});
INSERT INTO runners (id,pool_id,github_runner_id,name,status,os,architecture,provider,ci_platform,target_repository_id,created_at,updated_at) VALUES
 ('r-gh','pool-1',4242,'ios-builds-gh-1','idle','macos','arm64','tart','github',1103,${now},${now});
INSERT INTO runners (id,pool_id,name,status,os,architecture,provider,ci_platform,bitbucket_connection_id,created_at,updated_at) VALUES
 ('r-bb','pool-1','ios-builds-bb-1','idle','macos','arm64','tart','bitbucket','bb-1',${now},${now});
`;
}
execFileSync("sqlite3", [db], { input: sql, stdio: ["pipe", "inherit", "inherit"] });
const cookie = (t) => `${t}.${sign(t)}`;
console.log(JSON.stringify({ admin: cookie(tokens.admin).length, member: cookie(tokens.member).length }));
