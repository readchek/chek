import { defineConfig } from "cf/config"

export default defineConfig({
  accountId: "bf288a8d0e450c22157076593bc1f6be",
  worker: {
    name: "chek-website",
    compatibilityDate: "2026-10-08",
    domains: ["chek.page"],
  },
})
