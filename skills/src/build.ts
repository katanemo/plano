#!/usr/bin/env node

import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

type Section = {
  prefix: string;
  number: number;
  title: string;
  description: string;
};

type Rule = {
  file: string;
  title: string;
  impact: string;
  impactDescription: string;
  tags: string[];
  body: string;
  section: Section;
};

type ParsedFrontmatter = {
  frontmatter: Record<string, string>;
  body: string;
};

type Metadata = {
  abstract: string;
  version: string;
  organization: string;
};

const __dirname = dirname(fileURLToPath(import.meta.url));
const RULES_DIR = join(__dirname, "..", "rules");
const OUTPUT_FILE = join(__dirname, "..", "AGENTS.md");
const METADATA_FILE = join(__dirname, "..", "metadata.json");

const SECTIONS: Section[] = [
  {
    prefix: "config-",
    number: 1,
    title: "Configuration Fundamentals",
    description:
      "Core config.yaml structure, versioning, listener types, and provider setup — the entry point for every Plano deployment.",
  },
  {
    prefix: "routing-",
    number: 2,
    title: "Routing & Model Selection",
    description:
      "Intelligent LLM routing using preferences, aliases, and defaults to match tasks to the best model.",
  },
  {
    prefix: "agent-",
    number: 3,
    title: "Agent Orchestration",
    description:
      "Multi-agent patterns, agent descriptions, and orchestration strategies for building agentic applications.",
  },
  {
    prefix: "filter-",
    number: 4,
    title: "Filter Chains & Guardrails",
    description:
      "Request/response processing pipelines — ordering, MCP integration, and safety guardrails.",
  },
  {
    prefix: "observe-",
    number: 5,
    title: "Observability & Debugging",
    description:
      "OpenTelemetry tracing, log levels, span attributes, and sampling for production visibility.",
  },
  {
    prefix: "cli-",
    number: 6,
    title: "CLI Operations",
    description:
      "Using the planoai CLI for startup, tracing, CLI agents, project init, and code generation.",
  },
  {
    prefix: "deploy-",
    number: 7,
    title: "Deployment & Security",
    description:
      "Docker deployment, environment variable management, health checks, and state storage for production.",
  },
  {
    prefix: "advanced-",
    number: 8,
    title: "Advanced Patterns",
    description:
      "Prompt targets, external API integration, rate limiting, and multi-listener architectures.",
  },
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

  return {
    frontmatter,
    body: match[2].trim(),
  };
}

function inferSection(filename: string): Section | null {
  for (const section of SECTIONS) {
    if (filename.startsWith(section.prefix)) {
      return section;
    }
  }
  return null;
}

function main(): void {
  const metadata = JSON.parse(readFileSync(METADATA_FILE, "utf-8")) as Metadata;

  const files = readdirSync(RULES_DIR)
    .filter((f) => f.endsWith(".md") && !f.startsWith("_"))
    .sort();

  const sectionRules = new Map<number, Rule[]>();
  for (const section of SECTIONS) {
    sectionRules.set(section.number, []);
  }

  let parseErrors = 0;

  for (const file of files) {
    const content = readFileSync(join(RULES_DIR, file), "utf-8");
    const parsed = parseFrontmatter(content);

    if (!parsed) {
      console.error(`ERROR: Could not parse frontmatter in ${file}`);
      parseErrors++;
      continue;
    }

    const section = inferSection(file);
    if (!section) {
      console.warn(`WARN: No section found for ${file} — skipping`);
      continue;
    }

    const rule: Rule = {
      file,
      title: parsed.frontmatter.title ?? file,
      impact: parsed.frontmatter.impact ?? "MEDIUM",
      impactDescription: parsed.frontmatter.impactDescription ?? "",
      tags: parsed.frontmatter.tags
        ? parsed.frontmatter.tags.split(",").map((t) => t.trim())
        : [],
      body: parsed.body,
      section,
    };
    sectionRules.get(section.number)?.push(rule);
  }

  if (parseErrors > 0) {
    console.error(`\nBuild failed: ${parseErrors} file(s) had parse errors.`);
    process.exit(1);
  }

  for (const [, rules] of sectionRules) {
    rules.sort((a, b) => a.title.localeCompare(b.title));
  }

  const lines: string[] = [];
  lines.push(`# Plano Agent Skills`);
  lines.push(``);
  lines.push(`> ${metadata.abstract}`);
  lines.push(``);
  lines.push(
    `**Version:** ${metadata.version} | **Organization:** ${metadata.organization}`
  );
  lines.push(``);
  lines.push(`---`);
  lines.push(``);

  lines.push(`## Table of Contents`);
  lines.push(``);
  for (const section of SECTIONS) {
    const rules = sectionRules.get(section.number) ?? [];
    if (rules.length === 0) continue;
    lines.push(
      `- [Section ${section.number}: ${section.title}](#section-${section.number})`
    );
    for (let i = 0; i < rules.length; i++) {
      const rule = rules[i];
      const id = `${section.number}.${i + 1}`;
      const anchor = rule.title
        .toLowerCase()
        .replace(/[^a-z0-9\s-]/g, "")
        .replace(/\s+/g, "-");
      lines.push(`  - [${id} ${rule.title}](#${anchor})`);
    }
  }
  lines.push(``);
  lines.push(`---`);
  lines.push(``);

  for (const section of SECTIONS) {
    const rules = sectionRules.get(section.number) ?? [];
    if (rules.length === 0) continue;

    lines.push(`## Section ${section.number}: ${section.title}`);
    lines.push(``);
    lines.push(`*${section.description}*`);
    lines.push(``);

    for (let i = 0; i < rules.length; i++) {
      const rule = rules[i];
      const id = `${section.number}.${i + 1}`;

      lines.push(`### ${id} ${rule.title}`);
      lines.push(``);
      lines.push(
        `**Impact:** \`${rule.impact}\`${rule.impactDescription ? ` — ${rule.impactDescription}` : ""}`
      );
      if (rule.tags.length > 0) {
        lines.push(`**Tags:** ${rule.tags.map((t) => `\`${t}\``).join(", ")}`);
      }
      lines.push(``);
      lines.push(rule.body);
      lines.push(``);
      lines.push(`---`);
      lines.push(``);
    }
  }

  lines.push(`*Generated from individual rule files in \`rules/\`.*`);
  lines.push(
    `*To contribute, see [CONTRIBUTING](https://github.com/katanemo/archgw/blob/main/CONTRIBUTING.md).*`
  );

  writeFileSync(OUTPUT_FILE, lines.join("\n"), "utf-8");

  let totalRules = 0;
  for (const section of SECTIONS) {
    const rules = sectionRules.get(section.number) ?? [];
    if (rules.length > 0) {
      console.log(`  Section ${section.number}: ${rules.length} rules`);
      totalRules += rules.length;
    }
  }
  console.log(`\nBuilt AGENTS.md with ${totalRules} rules.`);
}

main();
