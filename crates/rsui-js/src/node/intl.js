'use strict';
// `Intl`, as far as programs and packages use it, on top of what `Number` and `Date` can already do for any language.
// None of this is compiled until something looks for it. Rules for plurals and the words in lists and times are English's.

function firstLocale(locales) {
    const locale = Array.isArray(locales) ? locales[0] : locales;
    return locale === undefined ? new NumberFormat().resolvedOptions().locale : String(locale);
}

const systemLocale = () => __fs.locale();

// The digits a locale and options ask for: `numberingSystem`, or the locale's `-u-nu-`, as ICU has them, else Latin.
const NUMBERING = new Set(['latn', 'arab', 'arabext', 'bali', 'beng', 'deva', 'fullwide', 'gujr', 'guru', 'hanidec', 'khmr',
    'knda', 'laoo', 'limb', 'mlym', 'mong', 'mymr', 'orya', 'tamldec', 'telu', 'thai', 'tibt']);
function numberingOf(locale, options) {
    const asked = options.numberingSystem ?? /-u(?:-[a-z0-9]+)*?-nu-([a-z0-9]+)/i.exec(String(locale ?? ''))?.[1]?.toLowerCase();
    return asked !== undefined && NUMBERING.has(String(asked)) ? String(asked) : 'latn';
}

class NumberFormat {
    #locale;
    #options;
    constructor(locales, options = {}) {
        const locale = Array.isArray(locales) ? locales[0] : locales;
        this.#locale = locale;
        this.#options = { ...options };
        // Bad options are refused when this is made, rather than when it's used.
        (0).toLocaleString(locale, this.#options);
        this.format = (value) => {
            if (typeof value === 'bigint') {
                return value.toLocaleString(this.#locale, this.#options);
            }
            return Number(value).toLocaleString(this.#locale, this.#options);
        };
    }

    formatToParts(value) {
        const text = this.format(value);
        const parts = [];
        const add = (type, piece) => {
            if (piece !== '') {
                parts.push({ type, value: piece });
            }
        };
        const pieces = text.match(/\d+|[^\d]+/g) ?? [];
        let beforeDecimal = true;
        const decimal = (1.5).toLocaleString(this.#locale, { useGrouping: false }).replace(/\d/g, '');
        pieces.forEach((piece, index) => {
            if (/^\d/.test(piece)) {
                add(beforeDecimal ? 'integer' : 'fraction', piece);
            } else if (index > 0 && index < pieces.length - 1 && beforeDecimal && piece === decimal) {
                beforeDecimal = false;
                add('decimal', piece);
            } else if (index > 0 && index < pieces.length - 1 && beforeDecimal) {
                add('group', piece);
            } else if (piece === '-') {
                add('minusSign', piece);
            } else if (piece.includes('%')) {
                add('percentSign', piece);
            } else {
                add(this.#options.style === 'currency' ? 'currency' : 'literal', piece);
            }
        });
        return parts;
    }

    resolvedOptions() {
        const style = this.#options.style ?? 'decimal';
        const minimum = this.#options.minimumFractionDigits ?? (style === 'currency' ? 2 : 0);
        return {
            locale: this.#locale ?? systemLocale(),
            numberingSystem: numberingOf(this.#locale, this.#options),
            style,
            minimumIntegerDigits: this.#options.minimumIntegerDigits ?? 1,
            minimumFractionDigits: minimum,
            maximumFractionDigits: this.#options.maximumFractionDigits ?? Math.max(minimum, style === 'decimal' ? 3 : style === 'currency' ? 2 : 0),
            useGrouping: this.#options.useGrouping ?? true,
            notation: this.#options.notation ?? 'standard',
            ...(style === 'currency' ? { currency: this.#options.currency } : {}),
        };
    }

    static supportedLocalesOf(locales) {
        return Array.isArray(locales) ? [...locales] : locales === undefined ? [] : [locales];
    }
}

const DATE_PARTS = ['weekday', 'year', 'month', 'day'];
const TIME_PARTS = ['hour', 'minute', 'second'];

function zoneName() {
    const minutes = -new Date().getTimezoneOffset();
    if (minutes === 0) {
        return 'UTC';
    }
    // These run the other way around, which is how they've always been. It's the closest there is without the names of places.
    const hours = Math.trunc(minutes / 60);
    return minutes % 60 === 0 ? `Etc/GMT${hours > 0 ? '-' : '+'}${Math.abs(hours)}` : 'UTC';
}

class DateTimeFormat {
    #locale;
    #options;
    constructor(locales, options = {}) {
        const locale = Array.isArray(locales) ? locales[0] : locales;
        this.#locale = locale;
        this.#options = { ...options };
        this.format = (value) => {
            const date = value === undefined ? new Date() : new Date(value);
            if (Number.isNaN(date.getTime())) {
                throw new RangeError('Invalid time value');
            }
            const given = this.#options;
            const any = [...DATE_PARTS, ...TIME_PARTS, 'dateStyle', 'timeStyle'].some((name) => given[name] !== undefined);
            // With nothing asked for, it's only the date.
            return any ? date.toLocaleString(this.#locale, given) : date.toLocaleDateString(this.#locale, given);
        };
    }

    // Each part's found by writing it on its own and looking for that in the whole, in the order they're likely to be in.
    formatToParts(value) {
        const date = value === undefined ? new Date() : new Date(value);
        const whole = this.format(date);
        const given = this.#options;
        const any = [...DATE_PARTS, ...TIME_PARTS].some((name) => given[name] !== undefined);
        const asked = any ? given : { year: 'numeric', month: 'numeric', day: 'numeric' };
        const found = [];
        const taken = new Array(whole.length).fill(false);
        for (const type of [...DATE_PARTS, ...TIME_PARTS, 'dayPeriod']) {
            let text;
            if (type === 'dayPeriod') {
                text = /[AaPp]\.?[Mm]\.?/.exec(whole)?.[0];
            } else if (asked[type] !== undefined) {
                const alone = { [type]: asked[type], timeZone: given.timeZone };
                if (type === 'hour' && given.hour12 !== undefined) {
                    alone.hour12 = given.hour12;
                }
                text = date.toLocaleString(this.#locale, alone);
                if (type === 'hour') {
                    text = /\d+/.exec(text)?.[0];
                } else if (type === 'minute' || type === 'second') {
                    text = text.padStart(asked[type] === '2-digit' || type === 'second' || type === 'minute' ? 2 : 1, '0');
                }
            }
            if (!text) {
                continue;
            }
            let at = whole.indexOf(text);
            while (at !== -1 && taken.slice(at, at + text.length).some(Boolean)) {
                at = whole.indexOf(text, at + 1);
            }
            if (at !== -1) {
                taken.fill(true, at, at + text.length);
                found.push({ type, value: text, at });
            }
        }
        found.sort((a, b) => a.at - b.at);
        const parts = [];
        let upTo = 0;
        for (const { type, value: text, at } of found) {
            if (at > upTo) {
                parts.push({ type: 'literal', value: whole.slice(upTo, at) });
            }
            parts.push({ type, value: text });
            upTo = at + text.length;
        }
        if (upTo < whole.length) {
            parts.push({ type: 'literal', value: whole.slice(upTo) });
        }
        return parts;
    }

    resolvedOptions() {
        return {
            locale: this.#locale === undefined || /^default$/i.test(this.#locale) ? systemLocale() : this.#locale,
            calendar: 'gregory',
            timeZone: this.#options.timeZone ?? zoneName(),
            ...this.#options,
            numberingSystem: numberingOf(this.#locale, this.#options),
        };
    }

    static supportedLocalesOf(locales) {
        return Array.isArray(locales) ? [...locales] : locales === undefined ? [] : [locales];
    }
}

// Comparing numbers in text by what they are, so that `2` goes before `10`, if that's asked for.
function compareNumbers(a, b, compareText) {
    const pieces = /\d+|\D+/g;
    const first = a.match(pieces) ?? [];
    const second = b.match(pieces) ?? [];
    for (let at = 0; at < Math.min(first.length, second.length); at += 1) {
        const [x, y] = [first[at], second[at]];
        const order = /^\d/.test(x) && /^\d/.test(y) ? Number(x) - Number(y) : compareText(x, y);
        if (order !== 0) {
            return Math.sign(order);
        }
    }
    return Math.sign(first.length - second.length);
}

class Collator {
    constructor(locales, options = {}) {
        const locale = Array.isArray(locales) ? locales[0] : locales;
        const loose = options.sensitivity === 'base' || options.sensitivity === 'accent';
        const plain = (text) => {
            const folded = loose ? text.toLowerCase() : text;
            return options.sensitivity === 'base' ? folded.normalize('NFD').replace(/[̀-ͯ]/g, '') : folded;
        };
        const compareText = (a, b) => plain(a).localeCompare(plain(b), locale);
        this.compare = (a, b) => {
            const [x, y] = [String(a), String(b)];
            return options.numeric ? compareNumbers(x, y, compareText) : compareText(x, y);
        };
        this.resolvedOptions = () => ({
            locale: locale ?? systemLocale(),
            usage: options.usage ?? 'sort',
            sensitivity: options.sensitivity ?? 'variant',
            ignorePunctuation: false,
            collation: 'default',
            numeric: Boolean(options.numeric),
            caseFirst: 'false',
        });
    }

    static supportedLocalesOf(locales) {
        return Array.isArray(locales) ? [...locales] : locales === undefined ? [] : [locales];
    }
}

class PluralRules {
    #options;
    constructor(locales, options = {}) {
        this.#options = options;
    }

    select(value) {
        const number = Math.abs(Number(value));
        if (this.#options.type === 'ordinal') {
            const [last, lastTwo] = [number % 10, number % 100];
            if (last === 1 && lastTwo !== 11) {
                return 'one';
            }
            if (last === 2 && lastTwo !== 12) {
                return 'two';
            }
            return last === 3 && lastTwo !== 13 ? 'few' : 'other';
        }
        return number === 1 ? 'one' : 'other';
    }

    resolvedOptions() {
        return { locale: 'en', type: this.#options.type ?? 'cardinal', pluralCategories: ['one', 'other'] };
    }
}

class ListFormat {
    #word;
    constructor(locales, options = {}) {
        this.#word = options.type === 'disjunction' ? 'or' : options.type === 'unit' ? '' : 'and';
    }

    format(items) {
        const list = [...items].map(String);
        if (list.length < 2) {
            return list.join('');
        }
        if (this.#word === '') {
            return list.join(', ');
        }
        if (list.length === 2) {
            return `${list[0]} ${this.#word} ${list[1]}`;
        }
        return `${list.slice(0, -1).join(', ')}, ${this.#word} ${list[list.length - 1]}`;
    }
}

class RelativeTimeFormat {
    #numeric;
    constructor(locales, options = {}) {
        this.#numeric = options.numeric ?? 'always';
    }

    format(value, unit) {
        const name = String(unit).replace(/s$/, '');
        const number = Number(value);
        if (this.#numeric === 'auto') {
            const said = {
                day: { '-1': 'yesterday', 0: 'today', 1: 'tomorrow' },
                year: { '-1': 'last year', 0: 'this year', 1: 'next year' },
                month: { '-1': 'last month', 0: 'this month', 1: 'next month' },
                week: { '-1': 'last week', 0: 'this week', 1: 'next week' },
                hour: { 0: 'this hour' },
                minute: { 0: 'this minute' },
                second: { 0: 'now' },
            }[name]?.[number];
            if (said !== undefined) {
                return said;
            }
        }
        const amount = Math.abs(number);
        const words = `${amount.toLocaleString('en-US')} ${name}${amount === 1 ? '' : 's'}`;
        return number < 0 || Object.is(number, -0) ? `${words} ago` : `in ${words}`;
    }
}

const Intl = {
    NumberFormat,
    DateTimeFormat,
    Collator,
    PluralRules,
    ListFormat,
    RelativeTimeFormat,
    getCanonicalLocales(locales) {
        return locales === undefined ? [] : Array.isArray(locales) ? [...locales] : [locales];
    },
};
Object.defineProperty(Intl, Symbol.toStringTag, { value: 'Intl', configurable: true });

module.exports = { Intl, firstLocale };
