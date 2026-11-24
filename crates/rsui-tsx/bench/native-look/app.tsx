// A thousand buttons in the platform's look (`<button native>`) against a thousand styled by hand to look like them, for what
// `native` costs. The hand-styled ones say all that Material's say, apart from its ripple and focus ring. `NATIVE_LOOK_N` says
// how many (1,000 unless it says), `NATIVE_LOOK_MODE=plain` styles them by hand. Clicking the first counts, and every button
// says the count, so each click makes them all again. `run.ps1` runs both.
import { useState } from 'rsui';

export const title = 'Native look bench';
export const size = [900, 700];

const COUNT = Number(process.env.NATIVE_LOOK_N || 1000);
const PLAIN = process.env.NATIVE_LOOK_MODE === 'plain';
const INDEXES = Array.from({ length: COUNT }, (_, index) => index);

export default function App() {
    const [count, setCount] = useState(0);
    const bump = () => setCount(count + 1);
    return (
        <window native={!PLAIN} padding={12}>
            <scroll flex={1}>
                <row gap={8} flexWrap align="start">
                    {INDEXES.map((index) =>
                        PLAIN ? (
                            <button key={'b' + index} paddingX={24} paddingY={10} minHeight={40} minWidth={58} radius={20} background="#6750A4"
                                hover="#735EAB" pressed="#7662AD" color="#FFFFFF" fontFamily="Roboto" fontSize={14} fontWeight="500"
                                lineHeight={20} letterSpacing={0.1} textAlign="center" noWrap onClick={bump}>
                                {'Item ' + index + ' ' + count}
                            </button>
                        ) : (
                            <button key={'b' + index} native onClick={bump}>{'Item ' + index + ' ' + count}</button>
                        ),
                    )}
                </row>
            </scroll>
        </window>
    );
}
