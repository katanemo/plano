import { ImageResponse } from "next/og";
import { NextRequest } from "next/server";
import { client, urlFor } from "@/lib/sanity";

export const runtime = "edge";

const ALLOWED_HOSTS = new Set([
  "archgw-tau.vercel.app",
  "planoai.dev",
  "localhost",
]);

function getSafeBaseUrl(requestOrigin: string): string {
  if (process.env.NEXT_PUBLIC_APP_URL) {
    return process.env.NEXT_PUBLIC_APP_URL;
  }
  if (process.env.VERCEL_URL) {
    return `https://${process.env.VERCEL_URL}`;
  }
  try {
    const parsed = new URL(requestOrigin);
    if (ALLOWED_HOSTS.has(parsed.hostname)) {
      return parsed.origin;
    }
  } catch {}
  return "http://localhost:3000";
}

// Font loading function that uses the request origin
function loadFont(fileName: string, baseUrl: string) {
  return fetch(new URL(`/fonts/${fileName}`, baseUrl)).then((res) => {
    if (!res.ok) {
      throw new Error(
        `Failed to fetch font ${fileName}: ${res.status} ${res.statusText}`,
      );
    }
    return res.arrayBuffer();
  });
}

async function getBlogPost(slug: string) {
  const query = `*[_type == "blog" && slug.current == $slug && published == true][0] {
    _id,
    title,
    slug,
    summary,
    publishedAt,
    mainImage,
    author {
      name,
      title,
      image
    }
  }`;

  const post = await client.fetch(query, { slug });
  return post;
}

function formatDate(dateString: string): string {
  const date = new Date(dateString);
  const day = date.getDate();
  const month = date.toLocaleDateString("en-US", { month: "long" });
  const year = date.getFullYear();

  const getOrdinal = (n: number) => {
    const s = ["th", "st", "nd", "rd"];
    const v = n % 100;
    return n + (s[(v - 20) % 10] || s[v] || s[0]);
  };

  return `${month} ${getOrdinal(day)}, ${year}`;
}

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ slug: string }> },
) {
  try {
    // Get base URL for font loading - use validated origin
    const fontBaseUrl = getSafeBaseUrl(request.nextUrl.origin);

    // Load fonts with error handling
    let fontData;
    try {
      const [
        ibmPlexSans,
        jetbrainsMonoRegular,
        jetbrainsMonoMedium,
        jetbrainsMonoBold,
      ] = await Promise.all([
        loadFont("IBMPlexSans-VariableFont_wdth,wght.otf", fontBaseUrl),
        loadFont("JetBrainsMono-Regular.otf", fontBaseUrl),
        loadFont("JetBrainsMono-Medium.otf", fontBaseUrl),
        loadFont("jetbrains-mono-bold.otf", fontBaseUrl),
      ]).catch((error: Error) => {
        console.error("Error loading fonts:", error);
        throw new Error(`Failed to load fonts: ${error.message}`);
      });

      fontData = {
        ibmPlexSans,
        jetbrainsMonoRegular,
        jetbrainsMonoMedium,
        jetbrainsMonoBold,
      };
    } catch (error: unknown) {
      const errorMessage =
        error instanceof Error ? error.message : "Unknown error";
      console.error("Font loading error:", errorMessage);
      return new Response(
        JSON.stringify({
          error: "Failed to load required fonts",
          details: errorMessage,
          baseUrl: fontBaseUrl,
        }),
        { status: 500 },
      );
    }

    const { slug } = await params;
    const post = await getBlogPost(slug);

    if (!post) {
      return new Response(JSON.stringify({ error: "Post not found" }), {
        status: 404,
      });
    }

    // Get author image URL if available
    let authorImageUrl: string | null = null;
    if (post.author?.image) {
      authorImageUrl = urlFor(post.author.image).width(120).url();
    }

    // Use logo PNG
    const baseUrl = getSafeBaseUrl(request.nextUrl.origin);
    const logoUrl = `${baseUrl}/Logomark.png`;

    return new ImageResponse(
      <div
        style={{
          background: "linear-gradient(to top right, #ffffff, #dcdfff)",
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          padding: "60px 80px",
          position: "relative",
        }}
      >
        {/* Logo - Top Left - SVG as data URL */}
        <div
          style={{
            position: "absolute",
            top: "60px",
            left: "80px",
            display: "flex",
            alignItems: "center",
          }}
        >
          <img
            src={logoUrl}
            alt="Plano"
            width="120"
            height="48"
            style={{
              objectFit: "contain",
            }}
          />
        </div>

        {/* Main Content - Left-Aligned, aligned with logo */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "flex-start",
            justifyContent: "center",
            flex: 1,
            width: "85%",
            marginTop: "150px",
          }}
        >
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "flex-start",
              width: "100%",
            }}
          >
            {/* Title - Left Aligned */}
            <h1
              style={{
                fontSize: "64px",
                lineHeight: "1.1",
                color: "#000000",
                marginBottom: "24px",
                letterSpacing: "-0.08em",
                fontFamily: "IBM Plex Sans Bold",
                textAlign: "left",
              }}
            >
              {post.title}
            </h1>

            {/* Date - Below Title, Left Aligned */}
            {/* {post.publishedAt && (
              <div
                style={{
                  fontSize: "20px",
                  color: "#000000",
                  marginBottom: "40px",
                  letterSpacing: "-1.8px",
                  fontFamily: "IBM Plex Sans Regular",
                  textAlign: "left",
                }}
              >
                {formatDate(post.publishedAt)}
              </div>
            )} */}

            {/* Author Section - Below Date, Left Aligned */}
            {post.author?.name && (
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "16px",
                  marginTop: "20px",
                }}
              >
                {authorImageUrl && (
                  <img
                    src={authorImageUrl}
                    alt={post.author.name}
                    width="48"
                    height="48"
                    style={{
                      borderRadius: "4px",
                      objectFit: "cover",
                    }}
                  />
                )}
                <div
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "flex-start",
                  }}
                >
                  <div
                    style={{
                      fontSize: "20px",
                      color: "#7780d9",
                      textTransform: "uppercase",
                      letterSpacing: "0.09em",
                      fontFamily: "JetBrains Mono Bold",
                      textAlign: "left",
                    }}
                  >
                    {post.author.name}
                  </div>
                  {post.author.title && (
                    <div
                      style={{
                        fontSize: "14px",
                        color: "#28327D",
                        textTransform: "uppercase",
                        letterSpacing: "0.10em",
                        fontFamily: "JetBrains Mono Medium",
                        textAlign: "left",
                      }}
                    >
                      {post.author.title}
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
        </div>
      </div>,
      {
        width: 1200,
        height: 630,
        fonts: [
          {
            name: "IBM Plex Sans Regular",
            data: fontData.ibmPlexSans,
            style: "normal",
            weight: 400,
          },
          {
            name: "IBM Plex Sans Medium",
            data: fontData.ibmPlexSans,
            style: "normal",
            weight: 500,
          },
          {
            name: "IBM Plex Sans Bold",
            data: fontData.ibmPlexSans,
            style: "normal",
            weight: 700,
          },
          {
            name: "JetBrains Mono Regular",
            data: fontData.jetbrainsMonoRegular,
            style: "normal",
            weight: 400,
          },
          {
            name: "JetBrains Mono Medium",
            data: fontData.jetbrainsMonoMedium,
            style: "normal",
            weight: 500,
          },
          {
            name: "JetBrains Mono Bold",
            data: fontData.jetbrainsMonoBold,
            style: "normal",
            weight: 600,
          },
        ],
      },
    );
  } catch (error: unknown) {
    const errorMessage =
      error instanceof Error ? error.message : "Unknown error";
    console.error("Error generating image response:", error);
    return new Response(
      JSON.stringify({
        error: "Failed to generate image",
        details: errorMessage,
      }),
      { status: 500 },
    );
  }
}
