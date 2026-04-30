export const metadata = {
  title: "PlanoHelper",
  description: "Slack bot for Plano ops",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
