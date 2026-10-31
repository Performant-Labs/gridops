CREATE TABLE `api_tokens` (
  `id` text PRIMARY KEY NOT NULL,
  `user_id` text NOT NULL,
  `name` text NOT NULL,
  `verifier` text NOT NULL,
  `display_prefix` text NOT NULL,
  `scopes_json` text NOT NULL,
  `created_at` integer NOT NULL,
  `expires_at` integer NOT NULL,
  `last_used_at` integer,
  `revoked_at` integer,
  FOREIGN KEY (`user_id`) REFERENCES `users`(`id`) ON UPDATE no action ON DELETE CASCADE
);
--> statement-breakpoint
CREATE UNIQUE INDEX `api_tokens_verifier_unique` ON `api_tokens` (`verifier`);
--> statement-breakpoint
CREATE INDEX `api_tokens_user_id_idx` ON `api_tokens` (`user_id`);
--> statement-breakpoint
CREATE INDEX `api_tokens_user_active_idx` ON `api_tokens` (`user_id`) WHERE `revoked_at` IS NULL;
