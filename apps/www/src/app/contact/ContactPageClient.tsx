"use client";

import { useState } from "react";
import { Button } from "@katanemo/ui";
import { MessageSquare, Building2, MessagesSquare } from "lucide-react";

export default function ContactPageClient() {
  const [formData, setFormData] = useState({
    firstName: "",
    lastName: "",
    email: "",
    company: "",
    lookingFor: "",
    message: "",
  });
  const [status, setStatus] = useState<
    "idle" | "submitting" | "success" | "error"
  >("idle");
  const [errorMessage, setErrorMessage] = useState("");

  const handleChange = (
    e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>,
  ) => {
    const { name, value } = e.target;
    setFormData((prev) => ({ ...prev, [name]: value }));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setStatus("submitting");
    setErrorMessage("");

    try {
      const res = await fetch("/api/contact", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(formData),
      });

      const data = await res.json();

      if (!res.ok) {
        throw new Error(data.error || "Something went wrong");
      }

      setStatus("success");
      setFormData({
        firstName: "",
        lastName: "",
        email: "",
        company: "",
        lookingFor: "",
        message: "",
      });
    } catch (error) {
      setStatus("error");
      setErrorMessage(
        error instanceof Error ? error.message : "Failed to submit form",
      );
    }
  };

  return (
    <div className="flex flex-col min-h-screen">
      {/* Hero / Header Section */}
      <section className="pt-20 pb-16 px-4 sm:px-6 lg:px-8">
        <div className="max-w-324 mx-auto text-left">
          <h1 className="text-4xl sm:text-5xl lg:text-6xl font-normal leading-tight tracking-tighter text-black mb-6 text-left">
            <span className="font-sans">Let's start a </span>
            <span className="font-sans font-medium text-secondary">
              conversation
            </span>
          </h1>
          <p className="text-lg sm:text-xl text-black/60 max-w-2xl text-left font-sans">
            Whether you're an enterprise looking for a custom solution or a
            developer building cool agents, we'd love to hear from you.
          </p>
        </div>
      </section>

      {/* Main Content - Split Layout */}
      <section className="pb-24 px-4 sm:px-6 lg:px-8 grow">
        <div className="max-w-324 mx-auto">
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 lg:gap-8 items-stretch">
            {/* Left Side: Community (Discord) */}
            <div className="group relative bg-white rounded-2xl p-8 sm:p-10 flex flex-col justify-between h-full overflow-hidden">
              {/* Background icon */}
              <div className="absolute -top-4 -right-4 w-32 h-32 opacity-[0.03] group-hover:opacity-[0.06] transition-opacity duration-300">
                <MessagesSquare size={128} className="text-blue-600" />
              </div>

              <div className="relative z-10">
                <div className="relative z-10 mb-6">
                  <div className="inline-flex items-center gap-2 px-3.5 py-1.5 rounded-full bg-gray-100/80 backdrop-blur-sm text-gray-700 text-xs font-mono font-bold tracking-wider uppercase mb-6 w-fit border border-gray-200/50">
                    <MessageSquare size={12} className="text-gray-600" />
                    Community
                  </div>
                  <h2 className="text-3xl sm:text-4xl font-semibold tracking-tight text-gray-900">
                    Join Our Discord
                  </h2>
                </div>
                <p className="text-base sm:text-lg text-gray-600 mb-8 leading-relaxed max-w-md">
                  Connect with other developers, ask questions, share what
                  you're building, and stay updated on the latest features by
                  joining our Discord server.
                </p>
              </div>

              <div className="relative z-10 mt-auto">
                <Button asChild>
                  <a
                    href="https://discord.gg/pGZf2gcwEc"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    <MessageSquare size={18} />
                    Join Discord Server
                  </a>
                </Button>
              </div>
            </div>

            {/* Right Side: Enterprise Contact */}
            <div className="group relative bg-white rounded-2xl p-8 sm:p-10  h-full overflow-hidden">
              {/* Subtle background pattern */}
              <div className="absolute inset-0 bg-[linear-gradient(to_bottom_right,transparent_0%,rgba(0,0,0,0.01)_50%,transparent_100%)] opacity-0 group-hover:opacity-100 transition-opacity duration-500" />

              {/* Background icon */}
              <div className="absolute -top-4 -right-4 w-32 h-32 opacity-[0.08]">
                <Building2 size={128} className="text-gray-400" />
              </div>

              <div className="relative z-10 mb-8">
                <div className="inline-flex items-center gap-2 px-3.5 py-1.5 rounded-full bg-gray-100/80 backdrop-blur-sm text-gray-700 text-xs font-mono font-bold tracking-wider uppercase mb-6 w-fit border border-gray-200/50">
                  <Building2 size={12} className="text-gray-600" />
                  Enterprise
                </div>
                <h2 className="text-3xl sm:text-4xl font-semibold tracking-tight mb-4 text-gray-900">
                  Contact Us
                </h2>
              </div>

              <div className="relative z-10">
                {status === "success" ? (
                  <div className="bg-gradient-to-br from-green-50 to-emerald-50/50 rounded-xl p-8 text-center border border-green-200/50 shadow-sm">
                    <div className="inline-flex items-center justify-center w-16 h-16 rounded-full bg-green-100 mb-4 mx-auto">
                      <svg
                        className="w-8 h-8 text-green-600"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="currentColor"
                      >
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M5 13l4 4L19 7"
                        />
                      </svg>
                    </div>
                    <div className="text-green-700 text-xl font-semibold mb-2">
                      Message Sent!
                    </div>
                    <p className="text-gray-600 mb-6 text-sm">
                      Thank you for reaching out. We'll be in touch shortly.
                    </p>
                    <Button
                      variant="outline"
                      onClick={() => setStatus("idle")}
                      className="bg-white border-gray-200 hover:bg-gray-50"
                    >
                      Send another message
                    </Button>
                  </div>
                ) : (
                  <form onSubmit={handleSubmit} className="flex flex-col gap-5">
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                      <div className="flex flex-col gap-2">
                        <label
                          htmlFor="firstName"
                          className="text-sm font-medium text-gray-600"
                        >
                          First Name
                        </label>
                        <input
                          type="text"
                          id="firstName"
                          name="firstName"
                          required
                          value={formData.firstName}
                          onChange={handleChange}
                          className="flex h-11 w-full rounded-lg border border-gray-200 bg-white px-4 py-2.5 text-sm ring-offset-background placeholder:text-gray-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-secondary/20 focus-visible:border-secondary disabled:cursor-not-allowed disabled:opacity-50 transition-all shadow-sm hover:border-gray-300"
                          placeholder="Steve"
                        />
                      </div>
                      <div className="flex flex-col gap-2">
                        <label
                          htmlFor="lastName"
                          className="text-sm font-medium text-gray-600"
                        >
                          Last Name
                        </label>
                        <input
                          type="text"
                          id="lastName"
                          name="lastName"
                          required
                          value={formData.lastName}
                          onChange={handleChange}
                          className="flex h-11 w-full rounded-lg border border-gray-200 bg-white px-4 py-2.5 text-sm ring-offset-background placeholder:text-gray-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-secondary/20 focus-visible:border-secondary disabled:cursor-not-allowed disabled:opacity-50 transition-all shadow-sm hover:border-gray-300"
                          placeholder="Wozniak"
                        />
                      </div>
                    </div>

                    <div className="flex flex-col gap-2">
                      <label
                        htmlFor="email"
                        className="text-sm font-medium text-gray-600"
                      >
                        Work Email
                      </label>
                      <input
                        type="email"
                        id="email"
                        name="email"
                        required
                        value={formData.email}
                        onChange={handleChange}
                        className="flex h-11 w-full rounded-lg border border-gray-200 bg-white px-4 py-2.5 text-sm ring-offset-background placeholder:text-gray-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-secondary/20 focus-visible:border-secondary disabled:cursor-not-allowed disabled:opacity-50 transition-all shadow-sm hover:border-gray-300"
                        placeholder="steve@apple.com"
                      />
                    </div>

                    <div className="flex flex-col gap-2">
                      <label
                        htmlFor="company"
                        className="text-sm font-medium text-gray-600"
                      >
                        Company Name
                      </label>
                      <input
                        type="text"
                        id="company"
                        name="company"
                        value={formData.company}
                        onChange={handleChange}
                        className="flex h-11 w-full rounded-lg border border-gray-200 bg-white px-4 py-2.5 text-sm ring-offset-background placeholder:text-gray-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-secondary/20 focus-visible:border-secondary disabled:cursor-not-allowed disabled:opacity-50 transition-all shadow-sm hover:border-gray-300"
                        placeholder="Apple Inc."
                      />
                    </div>

                    <div className="flex flex-col gap-2">
                      <label
                        htmlFor="lookingFor"
                        className="text-sm font-medium text-gray-600"
                      >
                        How can we help?
                      </label>
                      <textarea
                        id="lookingFor"
                        name="lookingFor"
                        required
                        value={formData.lookingFor}
                        onChange={handleChange}
                        className="flex min-h-[120px] w-full rounded-lg border border-gray-200 bg-white px-4 py-3 text-sm ring-offset-background placeholder:text-gray-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-secondary/20 focus-visible:border-secondary disabled:cursor-not-allowed disabled:opacity-50 transition-all shadow-sm hover:border-gray-300 resize-none"
                        placeholder="Tell us about your use case, requirements, or questions..."
                      />
                    </div>

                    {errorMessage && (
                      <div className="text-red-700 text-sm bg-red-50 p-4 rounded-lg border border-red-200/50 shadow-sm">
                        <div className="font-medium mb-1">Error</div>
                        <div className="text-red-600">{errorMessage}</div>
                      </div>
                    )}

                    <div className="mt-1">
                      <Button
                        type="submit"
                        className="w-full"
                        size="lg"
                        disabled={status === "submitting"}
                      >
                        {status === "submitting"
                          ? "Sending..."
                          : "Send Message"}
                      </Button>
                    </div>
                  </form>
                )}
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
