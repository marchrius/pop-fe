# Translations

`en.json` contains the English source messages and `it.json` their Italian
translations. Both catalogs use English messages as stable keys.

To add a language, copy `en.json`, translate the values, and add the language
code to the supported codes in `psp_i18n.language()`. Keep `{placeholders}`
unchanged. Do not translate game IDs, filenames, theme IDs or backend config
keys. A missing translation falls back to the English source message.

Locale selection follows LANGUAGE (including ordered lists), then LC_ALL,
LC_MESSAGES and LANG. Unsupported locales default to English. Restart the
application after changing the language; child processes inherit the locale.
