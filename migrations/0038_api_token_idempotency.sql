ALTER TABLE `api_tokens` ADD `idempotency_key` text;
--> statement-breakpoint
CREATE UNIQUE INDEX `api_tokens_user_idempotency_key_unique` ON `api_tokens` (`user_id`, `idempotency_key`) WHERE `idempotency_key` IS NOT NULL;
