.. _agents:

Agents
======

Agents are autonomous systems that handle wide-ranging, open-ended tasks by calling models in a loop until the work is complete. Agents have access to tools, reason about which actions to take, and adapt their behavior based on intermediate results—making them ideal for complex workflows that require multi-step reasoning, external API calls, and dynamic decision-making.

Plano helps developers build and scale multi-agent systems by managing the orchestration layer—deciding which agent(s) or LLM(s) should handle each request, and in what sequence—while developers focus on implementing agent logic in any language or framework they choose.

Agent Orchestration
-------------------

**Plano-Orchestrator** is a family of state-of-the-art routing and orchestration models that decide which agent(s) should handle each request, and in what sequence. Built for real-world multi-agent deployments, it analyzes user intent and conversation context to make precise routing and orchestration decisions while remaining efficient enough for low-latency production use across general chat, coding, and long-context multi-turn conversations.

This allows development teams to:

* **Scale multi-agent systems**: Route requests across multiple specialized agents without hardcoding routing logic in application code.
* **Improve performance**: Direct requests to the most appropriate agent based on intent, reducing unnecessary handoffs and improving response quality.
* **Enhance debuggability**: Centralized routing decisions are observable through Plano's tracing and logging, making it easier to understand why a particular agent was selected.

Inner Loop vs. Outer Loop
--------------------------

Plano distinguishes between the **inner loop** (agent implementation logic) and the **outer loop** (orchestration and routing):

Inner Loop (Agent Logic)
^^^^^^^^^^^^^^^^^^^^^^^^^

The inner loop is where your agent lives—the business logic that decides which tools to call, how to interpret results, and when the task is complete. You implement this in any language or framework:

* **Python agents**: Using frameworks like LangChain, LlamaIndex, CrewAI, or custom Python code.
* **JavaScript/TypeScript agents**: Using frameworks like LangChain.js or custom Node.js implementations.
* **Any other AI famreowkr**: Agents are just HTTP services that Plano can route to.

Your agent controls:

* Which tools or APIs to call in response to a prompt.
* How to interpret tool results and decide next steps.
* When to call the LLM for reasoning or summarization.
* When the task is complete and what response to return.

.. note::
   **Making LLM Calls from Agents**

   When your agent needs to call an LLM for reasoning, summarization, or completion, you should route those calls through Plano's Model Proxy rather than calling LLM providers directly. This gives you:

   * **Consistent responses**: Normalized response formats across all :ref:`LLM providers <llm_providers>`, whether you're using OpenAI, Anthropic, Azure OpenAI, or any OpenAI-compatible provider.
   * **Rich agentic signals**: Automatic capture of function calls, tool usage, reasoning steps, and model behavior—surfaced through traces and metrics without instrumenting your agent code.
   * **Smart model routing**: Leverage :ref:`model-based, alias-based, or preference-aligned routing <llm_providers>` to dynamically select the best model for each task based on cost, performance, or custom policies.

   By routing LLM calls through the Model Proxy, your agents remain decoupled from specific providers and can benefit from centralized policy enforcement, observability, and intelligent routing—all managed in the outer loop. For a step-by-step guide, see :ref:`llm_router` in the LLM Router guide.

Outer Loop (Orchestration)
^^^^^^^^^^^^^^^^^^^^^^^^^^^

The outer loop is Plano's orchestration layer—it manages the lifecycle of requests across agents and LLMs:

* **Intent analysis**: Plano-Orchestrator analyzes incoming prompts to determine user intent and conversation context.
* **Routing decisions**: Routes requests to the appropriate agent(s) or LLM(s) based on capabilities, context, and availability.
* **Sequencing**: Determines whether multiple agents need to collaborate and in what order.
* **Lifecycle management**: Handles retries, failover, circuit breaking, and load balancing across agent instances.

By managing the outer loop, Plano allows you to:

* Add new agents without changing routing logic in existing agents.
* Run multiple versions or variants of agents for A/B testing or canary deployments.
* Apply consistent :ref:`filter chains <filter_chain>` (guardrails, context enrichment) before requests reach agents.
* Monitor and debug multi-agent workflows through centralized observability.

Key Benefits
------------

* **Language and framework agnostic**: Write agents in any language; Plano orchestrates them via HTTP.
* **Reduced complexity**: Agents focus on task logic; Plano handles routing, retries, and cross-cutting concerns.
* **Better observability**: Centralized tracing shows which agents were called, in what sequence, and why.
* **Easier scaling**: Add more agent instances or new agent types without refactoring existing code.
