# Novelty check (G0.10)

Searched on 13 Oct 2026 for "Nifty gamma exposure", "gamma exposure pinning expiry", and "dealer gamma hedging India index options NSE pinning", aimed at arXiv, SSRN and GitHub.

## Findings

| Kind | What exists | Tested? |
|---|---|---|
| Vendor dashboards | Live Nifty GEX charts (QuintalMind, StockMojo, JustTicks, Vtrender, FnOLab) | No. All use the US sign convention (dealers long calls, short puts) |
| Vendor case studies | Vtrender: GEX "predicted" the 30 Dec 2025 Nifty settlement within 5 points | One event, self-published, no statistics |
| Practitioner guides | Pinning and volatility-regime explanations, mostly ported from SPX/0DTE material | No |
| Academic | arXiv 2512.17923 (LLM detection of GEX patterns), not India-specific. SqueezeMetrics' GEX white paper (US) is the usual methodology reference | Not for NSE |

Several India-focused sources make the same point: NSE publishes no dealer inventory, so the US sign assumption cannot be checked directly. That is the gap S1 and S2 address.

## Conclusion

We found no open, pre-registered, out-of-sample test of GEX pinning or volatility claims on Nifty, and nothing comparing the US sign convention with participant-wise positioning. The claim "first open test" holds as far as this search reaches. Repeat the search before publishing.

Sources: [arXiv 2512.17923](https://arxiv.org/pdf/2512.17923), [Vtrender case study](https://vtrender.com/posts/how-gamma-exposure-predicted-nifty-settlement-with-pinpoint-accuracy), [JustTicks guide](https://justticks.in/learn/gamma-exposure-nifty), [QuintalMind](https://www.quintalmind.com/gamma-exposure/nifty), [FnOLab](https://fnolab.com/learn/gex-gamma-exposure), [StockMojo](https://stockmojo.in/gamma-exposure/nifty).
