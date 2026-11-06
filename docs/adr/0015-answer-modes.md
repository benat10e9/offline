# ADR 0015: Three answer modes in one conversation

Status: Accepted (2026-11-06)

## Decision

Every message goes through the same chat; the app picks the mode, the customer never does.

1. **Emergency cards** (bleeding, CPR, choking, shock, burns, heat, cold, carbon monoxide, snakebite, poisoning,
   seizure, stroke): a curated card is shown verbatim, instantly, never paraphrased by the model. No card: "call 911"
   and official passages to read.
2. **Strict, cited** (health and injury, medicines, drinking water and food safety, generators/fuel/electrical/gas,
   law and permits, and any question about a specific piece of the customer's equipment): answers only from library
   passages, every claim cited, Verified / Partly supported / Not in the library.
3. **Conversational** (everything else, including small talk): natural answers from the model's general knowledge,
   library passages offered and cited when used, labelled "General knowledge", never Verified.

Classification is deterministic (`needs_strict_mode`), errs toward strict, and for follow-ups looks at the earlier
question too, so "what about for kids?" after a water question stays strict.

## Why

The strict mode made every answer read like annotated excerpts, which is right for safety and wrong for everything
else. Customers expect a conversational assistant; safety topics still get verified sources or nothing.
