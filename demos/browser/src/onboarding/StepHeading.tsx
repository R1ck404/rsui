// What each step of the tour is about, and a line on why.

export function StepHeading(props: { title: string; detail: string }) {
    return (
        <>
            <h2 className="text-[26px] font-semibold tracking-[-0.02em] text-(--b-text)">{props.title}</h2>
            <p className="mt-2 text-[14px] text-(--b-muted)">{props.detail}</p>
        </>
    );
}
