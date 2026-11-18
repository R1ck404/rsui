// Privacy: whether history's kept, forgetting it, forgetting the open space's cookies, and the tour again.

import type { Settings } from '../model';
import { Toggle } from '../ui/controls';
import { Plain, Row } from './parts';

export function Privacy(props: {
    settings: Settings;
    historyCount: number;
    onSettings: (changes: Partial<Settings>) => void;
    onClearHistory: () => void;
    onClearCookies: () => void;
    onTour: () => void;
}) {
    const { settings, historyCount } = props;
    return (
        <>
            <Row title="Remember where you've been" detail="For suggestions in the command bar and on the start page. It stays on this computer.">
                <Toggle label="Remember history" on={settings.keepHistory} onChange={(keepHistory) => props.onSettings({ keepHistory })} />
            </Row>
            <Row title="History" detail={historyCount === 1 ? '1 page remembered.' : historyCount + ' pages remembered.'}>
                <Plain onClick={props.onClearHistory}>Clear history</Plain>
            </Row>
            <Row title="Cookies and sign-ins" detail="Forget them for the space that's open. Sites will ask you to sign in again.">
                <Plain onClick={props.onClearCookies}>Forget</Plain>
            </Row>
            <Row title="The tour" detail="The welcome you saw the first time.">
                <Plain onClick={props.onTour}>Show again</Plain>
            </Row>
        </>
    );
}
