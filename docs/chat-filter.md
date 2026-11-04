# Operator chat mutes

Automatic word matching, refusal logs, strikes, automatic mutes and automatic bans are removed from this fork. Chat, private messages, signs, anvil names and books are not checked against word lists. The native text-policy hook allows every text.

Operators retain `/mute`, `/tempmute`, `/mutelist`, `/unmute` and normal ban commands. The legacy `[chat_filter] enabled` option controls operator mute support; historical word-list and punishment settings are ignored. Existing mute records without an operator attribution (automatic mutes) are ignored when loaded. Existing ban files are not rewritten.
