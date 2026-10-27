/**
 * First-run boot orchestra seeding, shared by `playwright.config.ts` and the
 * specs that create their own browser contexts (`browser.newContext()` does NOT
 * inherit `use.storageState`).
 *
 * Why: every test gets a fresh browser profile, so without this the full-screen
 * boot orchestra (`src/components/boot/BootOrchestra.vue`) mounts in every spec
 * and swallows pointer events until its 10 s minimum expires. We seed the same
 * flag a real launch writes once it has been seen.
 */
export const BOOT_ORCHESTRA_SEEN_KEY = 'marguerite-boot-orchestra-seen'

export function bootOrchestraSeenStorage(origin: string) {
  return {
    cookies: [],
    origins: [
      {
        origin,
        localStorage: [{ name: BOOT_ORCHESTRA_SEEN_KEY, value: '1' }],
      },
    ],
  }
}
