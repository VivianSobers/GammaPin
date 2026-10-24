# Nifty Option Settlement

How NSE finally settles Nifty 50 index options (weekly and monthly), checked against official NSE, NSE Clearing (NCL), NSE Indices and SEBI documents on 13–14 October 2026.

The rule changed on **3 August 2026**, when NSE introduced the Closing Auction Session (CAS). The sample period straddles that date (see Implications).

## Summary

| Question | Answer | Source |
|---|---|---|
| 1. Final settlement price of Nifty options | The closing price of the Nifty 50 index in NSE's Capital Market segment on the expiry day. | NSE settlement-price page [S3]; NCL settlement-price page [S6]; SEBI Master Circular ch. 5, para 1.1.9 and 2.1.9 [S9], as amended by SEBI CAS circular para 4.9.1 [S8] |
| 2a. Index close, before 3 Aug 2026 | Free-float-weighted index computed from each constituent's closing price, where the closing price was the VWAP of the last 30 minutes (15:00–15:30). Fallbacks: no trade in the last 30 min → last traded price of the day; no trade all day → previous close. | NSE Indices FAQ [S15]; SEBI Master Circular 1.1.9 [S9]; SEBI CAS circular para 1 [S8]; NSE CM consolidated circular, 3.4 [S13] |
| 2b. Index close, from 3 Aug 2026 | Index computed from the constituents' closing prices, which for stocks with derivatives (all Nifty 50 stocks) are now the CAS equilibrium price (auction 15:15–15:35). Fallbacks: no equilibrium price → CAS reference price (VWAP 15:00–15:15, else LTP of the day, else previous close). If an index circuit closes the market early, no CAS is held and the old 30-min VWAP/LTP rule applies. | SEBI CAS circular 4.1, 4.3, 4.6, 4.9.1 [S8]; NSE CAS SOP [S10]; NSE CAS trading modalities, section C [S11]; Go-live circular [S12]; NSE Indices methodology, Nifty 50 criteria [S16] |
| 3. Mechanics | Cash-settled in INR, European exercise, automatic exercise of in-the-money positions at expiry, assigned randomly to shorts; final settlement paid/received on T+1 (T = expiry day). | NCL settlement mechanism [S5]; NSE contract pages [S1], [S4] |
| 4. Expiry day | Weekly: every Tuesday. Monthly: last Tuesday of the month. If the Tuesday is a trading holiday, the previous trading day. Applies to contracts expiring on or after 1 Sep 2025 (before that: Thursday). | NSE circular NSE/FAOP/68747 [S7]; NSE Nifty 50 F&O page [S1]; NSE contract specifications [S2] |
| 5. Our target vs. settlement | `Closing Index Value` for "Nifty 50" in `ind_close_all_DDMMYYYY.csv` equals NCL's published NIFTY option settlement price on all 6 expiries checked (21 Jul, 28 Jul, 4 Aug, 29 Sep, 6 Oct, 13 Oct 2026). The primary end-of-day design decides at the expiry eve's close, a full session before any closing input. The intraday 14:00 decision time (not run) is at least 60 minutes before any part of the closing computation starts (15:00 under both regimes). | NCL `FOSett_prce` files vs. NSE `ind_close_all` files [S17] |

## Details

### 1. Final settlement price

- **NSE settlement-price page** [S3] (updated 03/01/2023), under "Options Contracts on Index and Individual Securities – Final Exercise Settlement": "Index - Closing price of the relevant underlying index in the Capital Market segment of NSE, on the last trading day of the options contract."
- **NCL settlement-price page** [S6] (updated 07/05/2024): "Final settlement price for a Index derivative contract shall be based on the close price of the relevant underlying index in the Exchange on the last trading day of such contract or such other price as may be decided by the relevant authority from time to time."
- **SEBI Master Circular, chapter 5** [S9] (Dec 2024), para 1.1.9: "The settlement price shall be the closing price of the underlying index on the day of expiry. The closing price of the underlying index shall be based on last half an hour VWAP (Volume Weighted Average Price) of the constituents of the underlying index." Para 2.1.9 (index options): "Same as that for index future contracts as specified in clause 1.1.9."
- **SEBI CAS circular** [S8] (16 Jan 2026), para 4.9.1, replaces 1.1.9 with: "The settlement price shall be the closing price of the underlying index on the day of expiry. The closing price of the underlying index shall be based on the closing price of the constituents of the underlying index".
- **Bank Nifty monthly option spec** [S4] (updated 03/02/2021; its expiry-day text is outdated, but the settlement wording is the most explicit on any NSE spec page): "Final settlement price for an option contract shall be the closing price of the underlying index in the Normal Market of the Capital Market segment of National Stock Exchange on the last trading day…"

The Nifty 50 F&O page [S1] does not have its own "final settlement price" row. It refers to the general rule above.

### 2. How the Nifty 50 close is computed

**Before 3 August 2026 (30-minute VWAP).**
- NSE Indices FAQ [S15]: "How is the NIFTY 50 closing price calculated? The NIFTY closing prices are calculated by taking the last half an hour weighted average closing prices of the constituents of the index." The same FAQ says the close "is supposed to reflect the state of the stock market at 3:30 PM on NSE."
- SEBI CAS circular [S8], para 1: "Currently, the closing price of stocks in the equity cash segment … is determined on the basis of the Volume Weighted Average Price (VWAP) of trades executed during the last thirty minutes of the Continuous Trading Session (CTS)." Cash-market CTS ended at 15:30, so the window was 15:00–15:30.
- Stock-level fallback, NSE CM consolidated circular NSE/CMTR/67774 (30 Apr 2025), 3.4 [S13]: "The closing price is last half an hour value weighted average price. In case security is not traded in last half an hour, last traded price of the day is the close price. In case security is not traded for the day, the latest available close price is considered as the close price for that day."

**From 3 August 2026 (Closing Auction Session).**
- SEBI [S8] 4.1.1: "Initially, the closing price of stocks in the cash segment on which derivative contracts are available shall be determined based on CAS." 4.2.1: CAS is "a separate session of 20 minutes from 3:15 pm to 3:35 pm"; order entry closes randomly between 3:28 and 3:30 pm (4.2.2); derivatives trade until 3:40 pm (4.2.3). 4.6.2: "The equilibrium price shall be the price at which the maximum volume is executable." 4.6.6: "In case no equilibrium price is discovered, then reference price itself shall be the closing price."
- Reference price, SEBI [S8] 4.3.1 and NSE SOP [S10] 2.1–2.2: VWAP of trades from 3:00 to 3:15 pm; if none, the day's LTP; if no trade all day, the previous close.
- NSE trading-modalities circular NSE/CMTR/74466 (29 May 2026), section C [S11]: "For securities on which CAS is applicable, closing price shall be determined based on the equilibrium price determination mechanism in CAS. In case the equilibrium price is not determined, then reference price … shall be the close price." It also says: "In case market closes for the day before scheduled time due to Index circuit hit, the CAS session shall not be held … closing prices for all the securities shall be computed as per existing mechanism i.e. based on VWAP of trades in last 30 mins or LTP".
- Go-live: NSE/CMTR/75479 (30 Jul 2026) [S12]: CAS changes "shall be effective in LIVE from August 03, 2026."
- All Nifty 50 constituents are CAS stocks. NSE Indices methodology (Sept 2026) [S16], Nifty 50 criteria: "The constituents should have derivative contracts available on NSE." The NSE CAS FAQ [S14], Q8, adds that a stock keeps CAS eligibility while it has derivatives on any exchange.
- Index value during CAS, NSE SOP [S10] 6.5 and CAS FAQ Q7 [S14]: the "actual index value" uses "LTP of CTS" for CAS stocks, so it stops moving for those stocks after 15:15. The "indicative index close value" uses the "Indicative equilibrium price of the stock during CAS" (and, for non-CAS stocks, the "VWAP of all the trades from 3.00PM onwards").
- The NSE Indices methodology document itself (Nov 2019 and Sept 2026 editions) [S16] gives the index formula (free-float market cap / base mcap × base value) and says prices are "Regulated prices received from the National Stock Exchange of India Ltd". It does not define the constituent closing price. That definition comes from the exchange circulars above.

### 3. Settlement mechanics

- NCL settlement mechanism [S5], "Final Exercise Settlement": "Final Exercise settlement is effected for option positions at in-the-money strike prices existing at the close of trading hours, on the expiration day of an option contract. Long positions at in-the money strike prices are automatically assigned to short positions in option contracts with the same series, on a random basis." It continues: "For index options contracts … exercise style is European style. Final Exercise is Automatic on expiry of the option contracts." Then: "Exercise settlement is cash settled by debiting/ crediting of the clearing accounts…" and "Final settlement loss/ profit amount for option contracts on Index is debited/ credited … on T+1 day (T = expiry day)."
- Bank Nifty spec [S4]: "Final settlement will be Cash settled in INR based on final settlement price."
- Nifty 50 F&O page [S1]: "The options contracts are European style and physical settled". Index options cannot be physically delivered, and NCL [S5] says cash. "Physical settled" looks like an error on that page.
- Trading hours: from 3 Aug 2026 the equity derivatives segment trades until 15:40 (CAS FAQ Q4 [S14]; SEBI 4.2.3 [S8]). Option settlement is still the index close, not an option trade price.

### 4. Expiry day

- NSE/FAOP/68747 (25 Jun 2025) [S7]: NIFTY weekly contracts move from "Thursday of the week" to "Tuesday of the week", and monthly/quarterly/half-yearly contracts from "Last Thursday of expiry month" to "Last Tuesday of expiry month". This applies to "Newly generated contracts with expiry falling on/after September 01, 2025". The first Tuesday weekly was 02-Sep-25. Note: "Dates may be subject to change on account of any holiday declared".
- Nifty 50 F&O page [S1] (updated 23/09/2025): "Nifty 50 index options monthly contracts expire on the last Tuesday of the expiry month and weekly contracts expire on every Tuesday of the expiry week. If the Tuesday of the expiry period is a trading holiday, the contracts expire on the previous trading day."
- Contract specifications [S2] (updated 11/08/2026) say the same for Nifty 50: "Tuesday of the expiry period. If the Tuesday is a trading holiday, then the expiry day is the previous trading day."

## Implications for GammaPin

1. **The H1 target equals the final settlement price.** `research/settlements.py` reads the "Nifty 50" `Closing Index Value` from `ind_close_all_DDMMYYYY.csv`. That is NSE's published Capital Market close of the index, which is the settlement price by rule [S3, S6, S8]. We also checked the numbers directly against NCL's `FOSett_prce_DDMMYYYY.csv` (row `OPTIDX,NIFTY`):

   | Expiry | Type | Regime | `ind_close_all` | NCL OPTIDX NIFTY |
   |---|---|---|---|---|
   | 2026-07-21 | weekly | VWAP | 24187.70 | 24187.70 |
   | 2026-07-28 | monthly | VWAP | 23985.35 | 23985.35 |
   | 2026-08-04 | weekly | CAS | 24614.90 | 24614.90 |
   | 2026-09-29 | monthly | CAS | 22716.20 | 22716.20 |
   | 2026-10-06 | weekly | CAS | 22776.10 | 22776.10 |
   | 2026-10-13 | weekly | CAS | 22721.95 | 22721.95 |

   On 28 Jul and 29 Sep, NCL's expiring NIFTY future settled at the same value.

2. **No decision time overlaps the closing computation.** The primary end-of-day design decides at the close of the expiry eve, a full trading session before any input to the settlement price. The rest of this point concerns the intraday 14:00 design only (not run). Before 3 Aug 2026 the inputs were trades from 15:00 to 15:30. After 3 Aug 2026 they are the CAS reference VWAP (15:00–15:15) and the auction (15:15–15:35). The decision chain uses candles stamped ≤ 13:59, so no trade that enters the settlement price is visible at decision time. The gap is at least 60 minutes. A 14:00 forecast still has to cover 60–95 minutes of price movement plus the averaging or auction effect.

3. **The sample spans a change in how the target is formed.** Expiries up to 28 Jul 2026 settle on a 30-minute VWAP-based close. Expiries from 4 Aug 2026 settle on a CAS-based close. A VWAP close is smoothed. An auction close can be outside the day's continuous range: on 6 Oct and 13 Oct 2026 the Nifty close equalled the day's high. Pinning around a strike could behave differently under the two regimes.

4. **Index candles after 15:15 (inference).** Under CAS the disseminated "actual" Nifty value uses each CAS stock's last continuous-session trade [S10 6.5.2]. Since all Nifty 50 stocks are CAS stocks, the index should be flat from about 15:15 until the close prints. This was not tested on our candles.

**Compared with existing repo files** (described only; nothing edited):
- `research/settlements.py` docstring ("NSE index options settle on the index closing price of the expiry day"): consistent with the sources.
- `PREREGISTRATION.md` "Settlement" row (official `Closing Index Value` from `ind_close_all`): consistent and checked numerically above.
- `PREREGISTRATION.md` lists the 3 Aug 2026 CAS change under its known risks ("Closing Auction Session"). The H1 sample, 94 expiries from 2 Jan 2025 on the NSE bhavcopy, crosses it.
- `PREREGISTRATION.md` "Time to expiry" counts minutes to 15:30 with 375 minutes per session, and "Bars" drops candles outside 09:15–15:30. Since 3 Aug 2026, F&O trades until 15:40 and the settlement price is fixed by an auction that ends about 15:35. Using 15:30 is a reasonable proxy but no longer the exact end of price formation. This affects T slightly and does not affect H1 decision inputs. H2's last time-of-day bucket (15:15–15:30) may show near-zero index RV after 3 Aug 2026 if the index stops moving (point 4).
- `docs/CALENDAR.md` (Tuesday expiry, previous trading day on holidays): consistent.

## Unconfirmed

- **No NSE Indices document states the post-CAS Nifty close formula.** The rule is put together from SEBI 4.9.1 (index close based on constituents' closes) and NSE circulars (constituent close = CAS price). The NSE Indices FAQ [S15] still gives the pre-CAS 30-minute wording.
- **Joint settlement-price SOP not found.** SEBI [S8] 4.9.2 required a joint exchange/CC SOP on closing and settlement prices for index derivatives. The NCL settlement-price page [S6] was last updated 07/05/2024. The CAS FAQ [S14] Q56 says "For settlement price computations, kindly refer communications from NCL."
- **Pending SEBI consultation.** A SEBI consultation paper dated 12 Sep 2026, "Review of certain aspects of the Closing Auction Session, Market Timings and Settlement Methodologies for Derivative Contracts", was seen only as a listing title and was not read. It may change the settlement method again.
- **Revisions to published files.** Whether NSE ever revises `ind_close_all` after publication was not confirmed.
- **"Physical settled" wording.** The Nifty 50 F&O page [S1] says "physical settled". NCL [S5] says cash. NCL is taken as authoritative.
- **Exact pre-CAS window.** The 15:00–15:30 window comes from "last thirty minutes" of a CTS that ended at 15:30. No official text found states the clock times outright.

## Sources

All were read on 13–14 Oct 2026. NSE pages were fetched with a browser User-Agent.

- [S1] NSE, NIFTY 50 F&O product page (updated 23/09/2025): https://www.nseindia.com/static/products-services/equity-derivatives-nifty50
- [S2] NSE, Equity derivatives contract specifications (updated 11/08/2026): https://www.nseindia.com/static/products-services/equity-derivatives-contract-specifications
- [S3] NSE, Equity derivatives settlement price (updated 03/01/2023): https://www.nseindia.com/static/products-services/equity-derivatives-settlement-price
- [S4] NSE, Bank Nifty options contract specification – monthly (updated 03/02/2021): https://www.nseindia.com/static/products-services/equity-derivatives-bank-nifty-option-contract-specification-monthly
- [S5] NSE Clearing, Equity derivatives settlement mechanism: https://www.nseclearing.in/clearing-settlement/equity-derivatives/settlement-mechanism
- [S6] NSE Clearing, Equity derivatives settlement price (updated 07/05/2024): https://www.nseclearing.in/clearing-settlement/equity-derivatives/settlement-price
- [S7] NSE circular NSE/FAOP/68747, 25 Jun 2025, "Revision in Expiry Day of Index and Stock Derivatives Contracts - Update": https://nsearchives.nseindia.com/content/circulars/FAOP68747.pdf
- [S8] SEBI circular HO/47/11/11(3)2025-MRD-POD2/I/2765/2026, 16 Jan 2026, "Introduction of Closing Auction Session (CAS) in the Equity Cash Segment…": https://www.sebi.gov.in/sebi_data/attachdocs/jan-2026/1768576287344.pdf
- [S9] SEBI Master Circular for Stock Exchanges and Clearing Corporations, Chapter 5 Exchange Traded Derivatives (Dec 2024): https://www.sebi.gov.in/sebi_data/commondocs/dec-2024/RE_Chapter%205%20-%20Exchange%20Traded%20Derivatives%20FINAL_1_p.pdf
- [S10] NSE circular NSE/CMTR/73362, 18 Mar 2026, CAS operational guidelines and SOP: https://nsearchives.nseindia.com/content/circulars/CMTR73362.pdf
- [S11] NSE circular NSE/CMTR/74466, 29 May 2026, CAS trading modalities: https://nsearchives.nseindia.com/content/circulars/CMTR74466.zip
- [S12] NSE circular NSE/CMTR/75479, 30 Jul 2026, CAS go-live and mock trading: https://nsearchives.nseindia.com/content/circulars/CMTR75479.pdf
- [S13] NSE circular NSE/CMTR/67774, 30 Apr 2025, Capital Market consolidated circular (3.4 Daily Closing Price): https://nsearchives.nseindia.com/content/circulars/CMTR67774.zip
- [S14] NSE, FAQ on CAS in the Equity segment and changes in Equity Derivatives segment (v2.0, Jul 2026): https://nsearchives.nseindia.com//web/mediaattachment/2026-07/FAQs_on_Closing_Auction_Session_CAS_in_the_CM_and_changes_in_FO_Segment_ver1_20260706173436.pdf
- [S15] NSE Indices, Index concepts FAQ: https://www.niftyindices.com/resources/index-concepts/faqs
- [S16] NSE Indices, Methodology Document for Equity Indices (Sept 2026): https://www.niftyindices.com/Methodology/Method_NIFTY_Equity_Indices.pdf (also the Nov 2019 broad-market edition: https://nsearchives.nseindia.com/content/indices/Nifty_Broad_Market_Indices_Methodology.pdf)
- [S17] NCL settlement prices `https://nsearchives.nseindia.com/archives/nsccl/sett/FOSett_prce_DDMMYYYY.csv` and NSE index closes `https://nsearchives.nseindia.com/content/indices/ind_close_all_DDMMYYYY.csv`, for 21072026, 28072026, 04082026, 29092026, 06102026 and 13102026.
- Also read for context, not cited above: NSE/FAOP/73928 (28 Apr 2026, F&O consolidated circular, 1.5: option/future close = last half-hour VWAP).
