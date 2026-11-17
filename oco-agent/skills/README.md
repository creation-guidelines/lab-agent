# Skills: moved to grenudi/ideal-agent

The skills every agent shares (lab-session-start, land-a-change, record-the-work, and change-the-phone's
general part as change-a-device) moved on 2026-11-17 to grenudi/ideal-agent, the one home of general agent
know-how, with the methods of the roles (orchestrate-agents, research-a-feature, bring-up-a-part). They
ship there as a Claude Code plugin, so every machine and cloud session gets them, not only the lab:

    claude plugin marketplace add grenudi/ideal-agent
    claude plugin install ideal-agent@ideal-agent

What is one project's domain stays in that project (for linux-phone-oco: devlab and its guard in
lab-tools, the briefs in docs' prompts/). Why: lab-agent's decision 2026-11-17-skills-moved-to-ideal-agent,
and ideal-agent's 2026-11-17-skills-in-the-plugin.
