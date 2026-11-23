import { SettingsSection } from "~/components/settings-ui";

/**
 * The job category rules in Settings › General (#199), administrators only: the ordered rules
 * (`GET /api/v1/settings/job-categories`), each with its category, field and pattern, moved,
 * added and deleted in place, saved as a whole list through `src/lib/api.ts` (a 422's
 * `ApiError.details` shown in the row it names), and a live preview of how many recent jobs each
 * rule would classify.
 *
 * It renders inside the General settings form, so its buttons must be `type="button"` and it must
 * not open a `<form>` of its own.
 *
 * #197 renders a placeholder; #199 fills it in.
 */
export function JobCategoriesSection() {
  return (
    <SettingsSection description="How finished jobs are grouped on the fleet health page." title="Job categories">
      <p className="px-4 py-3 text-sm text-muted-foreground">Coming in #199</p>
    </SettingsSection>
  );
}
