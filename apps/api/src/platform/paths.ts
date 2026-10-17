import { isAbsolute, relative, sep } from "node:path"

export function containsPath(root: string, path: string): boolean {
  const child = relative(root, path)
  return child === "" || (!isAbsolute(child) && child !== ".." && !child.startsWith(`..${sep}`))
}
