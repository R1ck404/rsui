export type TriggerAction = 'highlight' | 'notify' | 'attention';

export interface TriggerRule {
    pattern: string;
    action: TriggerAction;
    ignoreCase: boolean;
    partial: boolean;
}

export function compileTrigger(rule: TriggerRule): RegExp | null {
    if (!rule.pattern || rule.pattern.length > 256) return null;
    try { return new RegExp(rule.pattern, rule.ignoreCase ? 'i' : ''); }
    catch { return null; }
}

export function isTriggerList(value: unknown): value is TriggerRule[] {
    return Array.isArray(value) && value.length <= 24 && value.every((rule) =>
        rule && typeof rule.pattern === 'string' && rule.pattern.length <= 256 &&
        ['highlight', 'notify', 'attention'].includes(rule.action) &&
        typeof rule.ignoreCase === 'boolean' && typeof rule.partial === 'boolean');
}
