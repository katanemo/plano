import { Resend } from "resend";
import { NextResponse } from "next/server";

function getResendClient() {
  const apiKey = process.env.RESEND_API_KEY;
  if (!apiKey) {
    throw new Error("RESEND_API_KEY environment variable is not set");
  }
  return new Resend(apiKey);
}

interface ContactPayload {
  email: string;
  firstName: string;
  lastName: string;
  company?: string;
  lookingFor: string;
}

function buildProperties(
  company?: string,
  lookingFor?: string,
): Record<string, string> | undefined {
  const properties: Record<string, string> = {};
  if (company) properties.company_name = company;
  if (lookingFor) properties.looking_for = lookingFor;
  return Object.keys(properties).length > 0 ? properties : undefined;
}

function isDuplicateError(error: {
  message?: string;
  statusCode?: number | null;
}): boolean {
  const errorMessage = error.message?.toLowerCase() || "";
  return (
    errorMessage.includes("already exists") ||
    errorMessage.includes("duplicate") ||
    error.statusCode === 409
  );
}

function createContactPayload(
  email: string,
  firstName: string,
  lastName: string,
  company?: string,
  lookingFor?: string,
) {
  const properties = buildProperties(company, lookingFor);
  return {
    email,
    firstName,
    lastName,
    unsubscribed: false,
    ...(properties && { properties }),
  };
}

export async function POST(req: Request) {
  try {
    const body = await req.json();
    const { firstName, lastName, email, company, lookingFor }: ContactPayload =
      body;

    if (!email || !firstName || !lastName || !lookingFor) {
      return NextResponse.json(
        { error: "Missing required fields" },
        { status: 400 },
      );
    }

    const contactPayload = createContactPayload(
      email,
      firstName,
      lastName,
      company,
      lookingFor,
    );
    const resend = getResendClient();

    const { data, error } = await resend.contacts.create(contactPayload);

    if (error) {
      if (isDuplicateError(error)) {
        const { data: updateData, error: updateError } =
          await resend.contacts.update(contactPayload);

        if (updateError) {
          console.error("Resend update error:", updateError);
          return NextResponse.json(
            { error: updateError.message || "Failed to update contact" },
            { status: 500 },
          );
        }

        return NextResponse.json({ success: true, data: updateData });
      }

      console.error("Resend create error:", error);
      return NextResponse.json(
        { error: error.message || "Failed to create contact" },
        { status: error.statusCode || 500 },
      );
    }

    return NextResponse.json({ success: true, data });
  } catch (error) {
    console.error("Unexpected error:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Unknown error" },
      { status: 500 },
    );
  }
}
