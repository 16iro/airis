import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

// Merge Tailwind utility classes safely (later classes override earlier ones).
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
