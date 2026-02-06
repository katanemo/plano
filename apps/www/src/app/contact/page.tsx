import type { Metadata } from "next";
import { pageMetadata } from "@/lib/metadata";
import ContactPageClient from "./ContactPageClient";

export const metadata: Metadata = pageMetadata.contact;

export default function ContactPage() {
  return <ContactPageClient />;
}
