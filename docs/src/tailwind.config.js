/** @type {import('tailwindcss').Config} */
const daisyui = require("../themes/goyo/src/daisyui.js");
const daisyTheme = require("../themes/goyo/src/daisyui-theme.js");
const goyoThemes = require("../themes/goyo/src/goyo-themes.js");
const customThemes = require("./goyo-themes.custom.js");

const daisyuiPlugin = daisyui.default || daisyui;
const themePlugin = daisyTheme.default || daisyTheme;

module.exports = {
    content: [
        "../themes/goyo/templates/**/*.html",
        "../content/**/*.md",
        "../themes/goyo/src/**/*.js",
        "../src/**/*.js",
    ],

    plugins: [
        // daisyUI core (all built-in themes available for the toggle)
        daisyuiPlugin({
            themes: "all",
        }),

        // Built-in Goyo themes
        themePlugin({
            name: "goyo-dark",
            "color-scheme": "dark",
            ...goyoThemes["goyo-dark"],
        }),
        themePlugin({
            name: "goyo-light",
            "color-scheme": "light",
            ...goyoThemes["goyo-light"],
        }),

        // Custom Bento themes
        themePlugin({
            name: "bento-light",
            ...customThemes["bento-light"],
        }),
        themePlugin({
            name: "bento-dark",
            ...customThemes["bento-dark"],
        }),
    ],
};
