// [g25] Yama ve oturum çatalı.

export function g25PromptSection(): string {
  return `# PATCH AND FORK
apply_patch edits several files in one call. The envelope is "*** Begin Patch" through "*** End Patch", with "*** Add File", "*** Delete File", and "*** Update File" sections. Context lines start with a space, removed lines with "-", added lines with "+". If a hunk does not match, nothing is written.
fork_session copies the current messages into a new saved chat and leaves this chat open. It does not switch the window.
Both are tools for parallel thinking, not ceremonies: fork when you want to try two approaches without polluting this chat, patch when several files change together. Say in one line what the patch does when it spans many files.`
}
