#!/usr/bin/env node

import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

type ParsedFrontmatter = {
  frontmatter: Record<string, string>;
  body: string;
};

type SectionPrefix = {
  prefix: string;
  number: number;
  title: string;
};

type ExampleExtraction = {
  incorrect: string | null;
  correct: string | null;
};

type TestCaseEntry = {
  id: string;
  section: number;
  sectionTitle: string;
  title: string;
  impact: string;
  tags: string[];
  testCase: {
    description: string;
    input: string | null;
    expected: string | null;
    evaluationPrompt: string;
  };
};

const __dirname = dirname(fileURLToPath(import.meta.url));
const RULES_DIR = join(__dirname, "..", "rules");
const OUTPUT_FILE = join(__dirname, "..", "test-cases.json");

const SECTION_PREFIXES: SectionPrefix[] = [
  { prefix: "config-", number: 1, title: "Configuration Fundamentals" },
  { prefix: "routing-", number: 2, title: "Routing & Model Selection" },
  { prefix: "agent-", number: 3, title: "Agent Orchestration" },
  { prefix: "filter-", number: 4, title: "Filter Chains & Guardrails" },
  { prefix: "observe-", number: 5, title: "Observability & Debugging" },
  { prefix: "cli-", number: 6, title: "CLI Operations" },
  { prefix: "deploy-", number: 7, title: "Deployment & Security" },
  { prefix: "advanced-", number: 8, title: "Advanced Patterns" },
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

function extractCodeBlocks(text: string): string[] {
  const blocks: string[] = [];
  const regex = /```(?:yaml|bash|python|typescript|json|sh)?\n([\s\S]*?)```/g;
  let match: RegExpExecArray | null;
  do {
    match = regex.exec(text);
    if (match) {
      blocks.push(match[1].trim());
    }
  } while (match !== null);
  return blocks;
}

function extractExamples(body: string): ExampleExtraction {
  const incorrectMatch = body.match(
    /\*\*Incorrect[^*]*\*\*[:\s]*([\s\S]*?)(?=\*\*Correct|\*\*Key|$)/
  );
  const correctMatch = body.match(
    /\*\*Correct[^*]*\*\*[:\s]*([\s\S]*?)(?=\*\*Incorrect|\*\*Key|\*\*Note|Reference:|$)/
  );

  return {
    incorrect: incorrectMatch
      ? extractCodeBlocks(incorrectMatch[1]).join("\n\n")
      : null,
    correct: correctMatch ? extractCodeBlocks(correctMatch[1]).join("\n\n") : null,
  };
}

function inferSection(filename: string): SectionPrefix | null {
  for (const s of SECTION_PREFIXES) {
    if (filename.startsWith(s.prefix)) return s;
  }
  return null;
}

function main(): void {
  const files = readdirSync(RULES_DIR)
    .filter((f) => f.endsWith(".md") && !f.startsWith("_"))
    .sort();

  const testCases: TestCaseEntry[] = [];

  for (const file of files) {
    const content = readFileSync(join(RULES_DIR, file), "utf-8");
    const parsed = parseFrontmatter(content);
    if (!parsed) continue;

    const { frontmatter, body } = parsed;
    const section = inferSection(file);
    if (!section) continue;

    const { incorrect, correct } = extractExamples(body);
    if (!incorrect && !correct) continue;

    testCases.push({
      id: file.replace(".md", ""),
      section: section.number,
      sectionTitle: section.title,
      title: frontmatter.title ?? file,
      impact: frontmatter.impact ?? "MEDIUM",
      tags: frontmatter.tags
        ? frontmatter.tags.split(",").map((t) => t.trim())
        : [],
      testCase: {
        description: `Detect and fix: "${frontmatter.title}"`,
        input: incorrect,
        expected: correct,
        evaluationPrompt: `Given the following Plano config or CLI usage, identify if it violates the rule "${frontmatter.title}" and explain how to fix it.`,
      },
    });
  }

  writeFileSync(OUTPUT_FILE, JSON.stringify(testCases, null, 2), "utf-8");
  console.log(`Extracted ${testCases.length} test cases to test-cases.json`);
}

main();
