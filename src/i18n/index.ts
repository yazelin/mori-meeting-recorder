import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zhTW from "./locales/zh-TW.json";
import en from "./locales/en.json";

const lang = navigator.language.startsWith("zh") ? "zh-TW" : "en";

i18n.use(initReactI18next).init({
  resources: { "zh-TW": { translation: zhTW }, en: { translation: en } },
  lng: lang,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;
