const BASE_URL = process.env.NEXT_PUBLIC_APP_URL || "https://katanemo.com";

export const siteConfig = {
  name: "Katanemo",
  tagline: "Forward-deployed AI infrastructure engineers",
  description:
    "Forward-deployed AI infrastructure engineers delivering industry-leading research and open-source technologies for agentic AI development efforts.",
  url: BASE_URL,
  ogImage: `${BASE_URL}/KatanemoLogo.svg`,
  links: {
    docs: "https://docs.katanemo.com",
    github: "https://github.com/katanemo/plano",
    discord: "https://discord.gg/pGZf2gcwEc",
    huggingface: "https://huggingface.co/katanemo",
  },
  keywords: [
    "Katanemo AI",
    "Katanemo",
    "Katanemo Labs",
  ],
  authors: [{ name: "Katanemo", url: "https://github.com/katanemo/plano" }],
  creator: "Katanemo",
};
