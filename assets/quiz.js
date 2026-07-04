/* Reusable quiz widget — KUKA MCP Learning Workspace */

function initQuiz(containerId, questions) {
  const container = document.getElementById(containerId);
  if (!container) return;

  let score = 0;
  let answered = 0;

  questions.forEach((q, qi) => {
    const qEl = document.createElement('div');
    qEl.className = 'quiz-question';

    const prompt = document.createElement('p');
    prompt.textContent = (qi + 1) + '. ' + q.question;
    qEl.appendChild(prompt);

    const opts = document.createElement('ul');
    opts.className = 'quiz-options';

    const feedback = document.createElement('div');
    feedback.className = 'quiz-feedback';

    q.options.forEach((opt, oi) => {
      const li = document.createElement('li');
      const btn = document.createElement('button');
      btn.textContent = opt;
      btn.addEventListener('click', function () {
        if (btn.disabled) return;
        // Disable all buttons for this question
        opts.querySelectorAll('button').forEach(b => b.disabled = true);
        answered++;
        if (oi === q.correct) {
          btn.classList.add('correct');
          feedback.textContent = q.explanation || 'Correct!';
          score++;
        } else {
          btn.classList.add('incorrect');
          opts.querySelectorAll('button')[q.correct].classList.add('correct');
          feedback.textContent = q.explanation || 'Not quite — see the highlighted answer.';
        }
        // Show score when all answered
        if (answered === questions.length) {
          scoreEl.style.display = 'block';
          scoreEl.textContent = `You got ${score} of ${questions.length} correct.`;
        }
      });
      li.appendChild(btn);
      opts.appendChild(li);
    });

    qEl.appendChild(opts);
    qEl.appendChild(feedback);
    container.appendChild(qEl);
  });

  const scoreEl = document.createElement('div');
  scoreEl.className = 'quiz-score';
  container.appendChild(scoreEl);
}
