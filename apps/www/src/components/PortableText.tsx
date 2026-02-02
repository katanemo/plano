"use client";

import { PortableText as SanityPortableText } from "@portabletext/react";
import Image from "next/image";
import { urlFor } from "@/lib/sanity";
import type { PortableTextBlock } from "@portabletext/types";
import { useState } from "react";
import ReactMarkdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneLight } from "react-syntax-highlighter/dist/esm/styles/prism";

interface PortableTextProps {
  content: PortableTextBlock[];
}

const codeTheme: any = oneLight;

function CodeBlock({
  code,
  language,
  filename,
  highlightedLines,
}: {
  code: string;
  language?: string;
  filename?: string;
  highlightedLines: Set<number>;
}) {
  const [copied, setCopied] = useState(false);
  const displayLanguage = language || "text";

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div className="my-6 lg:my-8 not-prose">
      <div className="rounded-xl border border-black/10 bg-white overflow-hidden shadow-sm">
        {(filename || language) && (
          <div className="flex items-center justify-between gap-4 px-4 py-2 text-xs font-semibold text-black/70 bg-black/4 border-b border-black/10">
            <span className="truncate text-black/80">{filename || "Code"}</span>
            <div className="flex items-center gap-2">
              <span className="uppercase tracking-wide text-black/60 font-mono">
                {displayLanguage}
              </span>
              <button
                type="button"
                onClick={handleCopy}
                className="inline-flex items-center justify-center h-6 w-6 rounded border border-black/10 text-black/60 hover:text-black hover:border-black/20 hover:bg-black/5 transition-colors"
                aria-label="Copy code"
                title={copied ? "Copied" : "Copy"}
              >
                {copied ? (
                  <svg
                    viewBox="0 0 20 20"
                    fill="currentColor"
                    className="h-3.5 w-3.5"
                  >
                    <title>Copied</title>
                    <path d="M16.704 5.296a1 1 0 010 1.414l-7.25 7.25a1 1 0 01-1.414 0l-3.25-3.25a1 1 0 011.414-1.414L8.25 11.343l6.543-6.547a1 1 0 011.411 0z" />
                  </svg>
                ) : (
                  <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    className="h-4 w-4"
                  >
                    <title>Copy</title>
                    <rect x="9" y="9" width="10" height="10" rx="2" />
                    <rect x="5" y="5" width="10" height="10" rx="2" />
                  </svg>
                )}
              </button>
            </div>
          </div>
        )}
        <SyntaxHighlighter
          language={displayLanguage}
          style={codeTheme}
          customStyle={{
            margin: 0,
            background: "transparent",
            padding: "1rem",
            fontSize: "0.875rem",
          }}
          lineProps={(lineNumber: number) =>
            highlightedLines.has(lineNumber)
              ? {
                  style: {
                    backgroundColor: "rgba(251, 191, 36, 0.2)",
                  },
                }
              : { style: {} }
          }
          wrapLines
          codeTagProps={{ style: { fontFamily: "inherit" } }}
        >
          {code}
        </SyntaxHighlighter>
      </div>
    </div>
  );
}

const markdownComponents: Components = {
  h1: (props) => (
    <h1 className="text-2xl font-semibold text-black mb-3" {...props} />
  ),
  h2: (props) => (
    <h2 className="text-xl font-semibold text-black mt-10 mb-5" {...props} />
  ),
  h3: (props) => (
    <h3 className="text-lg font-semibold text-black mt-10 mb-5" {...props} />
  ),
  p: (props) => (
    <p className="text-base text-black/80 mb-3 leading-relaxed" {...props} />
  ),
  ul: (props) => (
    <ul className="list-disc list-inside mb-3 text-black/80" {...props} />
  ),
  ol: (props) => (
    <ol className="list-decimal list-inside mb-3 text-black/80" {...props} />
  ),
  a: (props) => (
    <a className="text-secondary hover:underline font-medium" {...props} />
  ),
  blockquote: (props) => (
    <blockquote
      className="border-l-4 border-secondary pl-4 italic text-black/70"
      {...props}
    />
  ),
  table: (props) => (
    <div className="my-4 overflow-x-auto">
      <table
        className="w-full border-collapse text-sm text-left border border-black/20"
        {...props}
      />
    </div>
  ),
  thead: (props) => <thead className="bg-black/4" {...props} />,
  tbody: (props) => <tbody className="divide-y divide-black/10" {...props} />,
  tr: (props) => <tr className="even:bg-black/2" {...props} />,
  th: (props) => (
    <th className="border border-black/20 px-3 py-2 text-left font-semibold text-black" {...props} />
  ),
  td: (props) => (
    <td className="border border-black/20 px-3 py-2 text-left text-black/80" {...props} />
  ),
  pre: (props) => (
    <pre className="rounded bg-black/10 p-3 text-sm overflow-x-auto" {...props} />
  ),
  code: ({ className, children }) => {
    const match = /language-(\w+)/.exec(className || "");
    if (match) {
      return (
        <SyntaxHighlighter
          style={codeTheme}
          language={match[1]}
          PreTag="div"
          customStyle={{
            margin: 0,
            background: "transparent",
            fontSize: "0.875rem",
          }}
          codeTagProps={{ style: { fontFamily: "inherit" } }}
        >
          {String(children).replace(/\n$/, "")}
        </SyntaxHighlighter>
      );
    }

    return (
      <code
        className="rounded bg-black/10 px-1.5 py-0.5 text-[0.95em] font-mono"
      >
        {children}
      </code>
    );
  },
};

const components = {
  types: {
    code: ({ value }: any) => {
      if (!value?.code) return null;
      const highlightedLines = new Set<number>(
        Array.isArray(value.highlightedLines) ? value.highlightedLines : [],
      );
      return (
        <CodeBlock
          code={String(value.code)}
          language={value.language}
          filename={value.filename}
          highlightedLines={highlightedLines}
        />
      );
    },
    table: ({ value }: any) => {
      const rows = Array.isArray(value?.rows) ? value.rows : [];
      if (rows.length === 0) return null;
      const headerRowIndex = rows.findIndex((row: any) => row?.isHeader);
      const headerRow =
        headerRowIndex >= 0 ? rows[headerRowIndex] : undefined;
      const bodyRows =
        headerRowIndex >= 0
          ? rows.filter((_: any, index: number) => index !== headerRowIndex)
          : rows;
      const renderCells = (cells: any[], isHeader: boolean) =>
        (cells || []).map((cell: any, index: number) => {
          const Tag = isHeader ? "th" : "td";
          return (
            <Tag
              key={cell?._key || index}
              className={`border border-black/20 px-3 py-2 text-left align-top ${
                isHeader ? "font-semibold text-black" : "text-black/80"
              }`}
            >
              {cell?.value || ""}
            </Tag>
          );
        });

      return (
        <div className="my-6 lg:my-8 overflow-x-auto not-prose">
          <div className="rounded-xl border border-black/20 overflow-hidden bg-white">
            <table className="w-full border-collapse text-sm text-left">
            {headerRow?.cells?.length ? (
              <thead className="bg-black/4 text-black/80">
                <tr>{renderCells(headerRow.cells, true)}</tr>
              </thead>
            ) : null}
            <tbody className="divide-y divide-black/10">
              {bodyRows.map((row: any, rowIndex: number) => (
                <tr key={row?._key || rowIndex} className="even:bg-black/2">
                  {renderCells(row?.cells || [], false)}
                </tr>
              ))}
            </tbody>
            </table>
          </div>
        </div>
      );
    },
    markdownBlock: ({ value }: any) => {
      const markdown = value?.markdown;
      if (!markdown) return null;
      return (
        <div className="my-6 lg:my-8">
          <div className="rounded-xl border border-black/10 bg-black/2 px-5 py-4">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={markdownComponents}
            >
              {markdown}
            </ReactMarkdown>
          </div>
        </div>
      );
    },
    image: ({ value }: any) => {
      if (!value?.asset) return null;

      const imageUrl = urlFor(value);
      const asset = value.asset;

      // Get natural dimensions if available from metadata
      const dimensions = asset.metadata?.dimensions;
      const width = dimensions?.width || 1000;
      const height = dimensions?.height || 562;
      const aspectRatio = dimensions ? height / width : 0.5625; // Default to 16:9 if no dimensions

      return (
        <div className="my-6 lg:my-8">
          <div className="max-w-3xl mx-auto">
            <div className="relative w-full overflow-hidden rounded-lg bg-black/5">
              <div
                className="relative w-full"
                style={{ paddingBottom: `${aspectRatio * 100}%` }}
              >
                <Image
                  src={imageUrl.width(Math.min(width, 1000)).url()}
                  alt={value.alt || "Blog image"}
                  fill
                  className="object-contain"
                  sizes="(max-width: 768px) 100vw, (max-width: 1200px) 768px, 1000px"
                />
              </div>
            </div>
            {value.alt && (
              <p className="mt-2 text-sm text-black/60 text-center">
                {value.alt}
              </p>
            )}
          </div>
        </div>
      );
    },
  },
  block: {
    h1: (props: any) => (
      <h1 className="text-4xl sm:text-5xl lg:text-6xl font-normal leading-tight tracking-tighter text-black mt-8 mb-4 first:mt-0">
        <span className="font-sans">{props.children}</span>
      </h1>
    ),
    h2: (props: any) => (
      <h2 className="text-3xl sm:text-4xl lg:text-5xl font-normal leading-tight tracking-tighter text-black mt-10 mb-5 first:mt-0">
        <span className="font-sans">{props.children}</span>
      </h2>
    ),
    h3: (props: any) => (
      <h3 className="text-2xl sm:text-3xl lg:text-4xl font-normal leading-tight tracking-tighter text-black mt-8 mb-4 first:mt-0">
        <span className="font-sans">{props.children}</span>
      </h3>
    ),
    h4: (props: any) => (
      <h4 className="text-xl sm:text-2xl lg:text-3xl font-normal leading-tight tracking-tighter text-black mt-6 mb-3 first:mt-0">
        <span className="font-sans">{props.children}</span>
      </h4>
    ),
    normal: (props: any) => (
      <p className="text-base sm:text-lg font-sans font-[400] tracking-[-0.5px] text-black/80 mb-4 leading-relaxed">
        {props.children}
      </p>
    ),
    blockquote: (props: any) => (
      <blockquote className="border-l-4 border-[var(--secondary)] pl-6 py-2 my-6 italic text-black/70">
        {props.children}
      </blockquote>
    ),
  },
  list: {
    bullet: (props: any) => (
      <ul className="list-disc list-inside mb-4 space-y-2 text-base sm:text-lg font-sans font-[400] tracking-[-0.5px] text-black/80">
        {props.children}
      </ul>
    ),
    number: (props: any) => (
      <ol className="list-decimal list-inside mb-4 space-y-2 text-base sm:text-lg font-sans font-[400] tracking-[-0.5px] text-black/80">
        {props.children}
      </ol>
    ),
  },
  listItem: {
    bullet: (props: any) => <li className="ml-4">{props.children}</li>,
    number: (props: any) => <li className="ml-4">{props.children}</li>,
  },
  marks: {
    strong: ({ children }: { children: React.ReactNode }) => (
      <strong className="font-semibold text-black">{children}</strong>
    ),
    em: ({ children }: { children: React.ReactNode }) => (
      <em className="italic">{children}</em>
    ),
    link: (props: any) => (
      <a
        href={props.value?.href || "#"}
        target={props.value?.href?.startsWith("http") ? "_blank" : undefined}
        rel={
          props.value?.href?.startsWith("http")
            ? "noopener noreferrer"
            : undefined
        }
        className="text-[var(--secondary)] hover:underline font-medium"
      >
        {props.children}
      </a>
    ),
  },
};

export function PortableText({ content }: PortableTextProps) {
  return <SanityPortableText value={content} components={components} />;
}
