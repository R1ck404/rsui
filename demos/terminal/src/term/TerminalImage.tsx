import type { Image } from '../vt/terminal';
import type { Look } from './Row';

export function TerminalImage({ image, look, row }: { image: Image; look: Look; row: number }) {
    if (image.clip) {
        const { start, end } = image.clip;
        return (
            <div className="pointer-events-none absolute left-0 w-full overflow-hidden" style={{ top: (row + start) * look.ch, height: (end - start) * look.ch }}>
                <TerminalImage image={{ ...image, clip: undefined }} look={look} row={-start} />
            </div>
        );
    }
    const src = `data:${image.mime};base64,${image.data}`;
    const { viewport } = image;
    const left = look.gutter + image.col * look.cw + (viewport?.offsetX ?? 0);
    const top = row * look.ch + (viewport?.offsetY ?? 0);
    const width = image.pixels?.width ?? Math.max(1, image.cols * look.cw - (viewport?.offsetX ?? 0));
    const height = image.pixels?.height ?? Math.max(1, image.rows * look.ch - (viewport?.offsetY ?? 0));
    if (!viewport || (viewport.w === viewport.fullWidth && viewport.h === viewport.fullHeight)) {
        return <image position="absolute" src={src} fit={image.stretch ? 'fill' : 'contain'} left={left} top={top} width={width} height={height} />;
    }
    const scaleX = width / viewport.w;
    const scaleY = height / viewport.h;
    return (
        <div className="pointer-events-none absolute overflow-hidden" style={{ left, top, width, height }}>
            <image position="absolute" src={src} fit="fill" left={-viewport.x * scaleX} top={-viewport.y * scaleY}
                width={viewport.fullWidth * scaleX} height={viewport.fullHeight * scaleY} />
        </div>
    );
}
