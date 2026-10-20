type Props = {
  step: 1 | 2 | 3;
  uploaded?: boolean;
};

export function TestMatchProgress({ step, uploaded = false }: Props) {
  return <ol className="test-progress" aria-label="Test match progress">
    {['Get command', 'Save results', 'Upload'].map((label, index) => {
      const number = index + 1;
      const complete = number < step || uploaded;
      return <li key={label} className={complete ? 'is-complete' : number === step ? 'is-current' : undefined}
        aria-current={!uploaded && number === step ? 'step' : undefined}>
        <span className="test-progress-number" aria-hidden="true">{complete ? '✓' : number}</span>
        <span><span className="visually-hidden">{complete ? 'Completed: ' : number === step ? 'Next: ' : ''}</span>{label}</span>
      </li>;
    })}
  </ol>;
}
