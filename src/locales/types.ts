export type Language = "en" | "fr";

export type TranslationTree = {
  [key: string]: string | TranslationTree;
};
