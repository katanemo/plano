#!/usr/bin/env node

import { readFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

type ParsedFrontmatter = {
  frontmatter: Record<string, string>;
  body: string;
};

type ValidationResult = {
  errors: string[];
  warnings: string[];
};

const __dirname = dirname(fileURLToPath(import.meta.url));
const RULES_DIR = join(__dirname, "..", "rules");

const VALID_IMPACTS = [
  "CRITICAL",
  "HIGH",
  "MEDIUM-HIGH",
  "MEDIUM",
  "LOW-MEDIUM",
  "LOW",
] as const;

const SECTION_PREFIXES = [
  "config-",
  "routing-",
  "agent-",
  "filter-",
  "observe-",
  "cli-",
  "deploy-",
  "advanced-",
];

function parseFrontmatter(content: string): ParsedFrontmatter | null {
  const match = content.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
  if (!match) return null;

  const frontmatter: Record<string, string> = {};
  const lines = match[1].split("\n");
  for (const line of lines) {
    const colonIdx = line.indexOf(":");
    if (colonIdx === -1) continue;
    const key = line.slice(0, colonIdx).trim();
    const value = line.slice(colonIdx + 1).trim();
    frontmatter[key] = value;
  }

  return { frontmatter, body: match[2].trim() };
}

function validateFile(file: string, content: string): ValidationResult {
  const errors: string[] = [];
  const warnings: string[] = [];

  const parsed = parseFrontmatter(content);
  if (!parsed) {
    errors.push("Missing or malformed frontmatter (expected --- ... ---)");
    return { errors, warnings };
  }

  const { frontmatter, body } = parsed;

  if (!frontmatter.title) {
    errors.push("Missing required frontmatter field: title");
  }
  if (!frontmatter.impact) {
    errors.push("Missing required frontmatter field: impact");
  } else if (!VALID_IMPACTS.includes(frontmatter.impact as (typeof VALID_IMPACTS)[number])) {
    errors.push(
      `Invalid impact value: "${frontmatter.impact}". Valid values: ${VALID_IMPACTS.join(", ")}`
    );
  }
  if (!frontmatter.tags) {
    warnings.push("No tags defined — consider adding relevant tags");
  }

  const hasValidPrefix = SECTION_PREFIXES.some((p) => file.startsWith(p));
  if (!hasValidPrefix) {
    errors.push(
      `Filename must start with a valid prefix: ${SECTION_PREFIXES.join(", ")}`
    );
  }

  if (body.length < 100) {
    warnings.push("Rule body seems very short — consider adding more detail");
  }

  if (!body.includes("```")) {
    warnings.push(
      "No code examples found — rules should include YAML or CLI examples"
    );
  }

  if (!body.includes("Incorrect") || !body.includes("Correct")) {
    warnings.push(
      "Consider adding both Incorrect and Correct examples for clarity"
    );
  }

  return { errors, warnings };
}

function main(): void {
  const files = readdirSync(RULES_DIR)
    .filter((f) => f.endsWith(".md") && !f.startsWith("_"))
    .sort();

  let totalErrors = 0;
  let totalWarnings = 0;
  let filesWithIssues = 0;

  console.log(`Validating ${files.length} rule files...\n`);

  for (const file of files) {
    const content = readFileSync(join(RULES_DIR, file), "utf-8");
    const { errors, warnings } = validateFile(file, content);

    if (errors.length > 0 || warnings.length > 0) {
      filesWithIssues++;
      console.log(`📄 ${file}`);

      for (const error of errors) {
        console.log(`  ❌ ERROR: ${error}`);
        totalErrors++;
      }
      for (const warning of warnings) {
        console.log(`  ⚠️  WARN:  ${warning}`);
        totalWarnings++;
      }
      console.log();
    } else {
      console.log(`✅ ${file}`);
    }
  }

  console.log(`\n--- Validation Summary ---`);
  console.log(`Files checked:    ${files.length}`);
  console.log(`Files with issues: ${filesWithIssues}`);
  console.log(`Errors:           ${totalErrors}`);
  console.log(`Warnings:         ${totalWarnings}`);

  if (totalErrors > 0) {
    console.log(`\nValidation FAILED with ${totalErrors} error(s).`);
    process.exit(1);
  } else {
    console.log(`\nValidation passed.`);
  }
}

main();
