# Becoming a Decent Engineer (Who Writes Rust)

**Think before you type.**
Name the problem in plain English before opening your editor. If you can't explain what you're building in two sentences, you don't understand it well enough yet. This isn't a Rust thing. This is the whole job.

**Design your data before your logic.**
What does the data look like? What shape is it in at each stage? What transformations happen to it? Logic is just what you do to data — if the data model is wrong, no amount of clever logic saves you. Rust's type system will make you answer these questions anyway. Answer them before the compiler asks.

**Make illegal states unrepresentable.**
If it can't happen, encode that in the type. Don't use a boolean where an enum is what you mean. Don't use a string where a type is what you mean. Don't leave room for "impossible" states that you then have to defensively check for everywhere. This is a design principle, not a syntax preference.

**Understand the failure modes before writing the happy path.**
What can go wrong? What's missing? What times out? What's malformed? Most real programs are 20% happy path and 80% "what do we do when things aren't right." Think about that first, not last.

**Small, named, composable pieces.**
A function should do one thing and have a name that says what that thing is. If you're struggling to name it, it's doing too much. Composition — snapping well-defined pieces together — scales. Monolithic, tangled code doesn't. Rust rewards this heavily. So does every codebase you'll ever have to maintain.

**Slow down when it feels wrong.**
If you're contorting your code to make something work, that's signal. The design is probably wrong upstream. The instinct to push through and hack it into submission is almost always the wrong call. Back up. Rethink the model. Five minutes of thinking now saves hours of debugging later.

**Finish small things.**
Don't let "learning" become an excuse for a graveyard of half-built projects. Build something small enough to complete. Ship it, even to yourself. The act of finishing forces you to confront the parts you were avoiding.

**Read the error. The whole error.**
Not the first line. The whole thing. This applies to compiler errors, runtime panics, test failures, logs. The answer is almost always in there. The habit of actually reading — not skimming for a keyword to copy into a search engine — is rarer and more valuable than it sounds.

**Handle the unhappy path explicitly.**
Every unhandled error is a decision you're deferring to the worst possible moment. Make the decision now, deliberately, in the code. Rust makes this unavoidable with `Result` and `Option`. Internalize *why*, not just *how*.

**Read code that's better than yours.**
Not tutorials. Actual production code, standard libraries, well-regarded open source projects. See how experienced people name things, structure things, handle edge cases. You absorb taste by exposure.

---

The through-line: **good engineering is mostly clear thinking, and Rust is a particularly unforgiving place to think unclearly.** That's not a flaw. Use it.
