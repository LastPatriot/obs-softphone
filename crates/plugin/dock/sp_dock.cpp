// SPDX-License-Identifier: GPL-2.0-or-later
// The Call-In dock (DESIGN.md §5.1). The only C++ in the plugin: OBS docks
// must be Qt widgets. Plain widgets and lambdas, so no moc step is needed.
//
// Threads: sp_dock_create/sp_dock_shutdown run on the UI thread;
// sp_dock_update may be called from any thread and is queued to the UI thread.

#include <QtCore/QCoreApplication>
#include <QtCore/QElapsedTimer>
#include <QtCore/QMetaObject>
#include <QtCore/QPointer>
#include <QtCore/QTimer>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QVBoxLayout>
#include <QtWidgets/QWidget>

#include <QtCore/QThread>

#include <atomic>

extern "C" void blog(int log_level, const char *format, ...); // libobs
extern "C" void sp_chime_play(void);                         // sp_chime.c

extern "C" {
typedef void (*sp_dock_action_fn)(int action);

// Must match crates/plugin/src/dock.rs.
enum {
	SP_ACT_ANSWER = 1,
	SP_ACT_END = 2,
	SP_ACT_RETRY = 3,
	SP_ACT_TAKE_BACK = 4,
	SP_ACT_AUTO_ON = 5,
	SP_ACT_AUTO_OFF = 6,
	SP_ACT_FIX_MIXMINUS = 7,
	SP_ACT_MUTE_TOGGLE = 8,
	SP_ACT_SETTINGS = 9,
	SP_ACT_ADD_TO_SCENE = 10,
};
// The button beside a warning (crates/plugin/src/dock.rs WarnButton).
enum { SP_WARN_NONE = 0, SP_WARN_FIX = 1, SP_WARN_ADD_TO_SCENE = 2 };
enum { SP_ST_DISABLED, SP_ST_CONNECTING, SP_ST_READY, SP_ST_RINGING, SP_ST_ON_AIR, SP_ST_ERROR, SP_ST_REPLACED, SP_ST_SETUP };

void *sp_dock_create(sp_dock_action_fn on_action, const char *footer);
void sp_dock_update(int status, const char *headline, const char *caller, const char *message, int auto_answer);
void sp_dock_shutdown(void);
void sp_dock_set_warning(const char *text, int button);
void sp_dock_set_muted(int muted);
void sp_dock_set_chime(int on);
}

namespace {

struct State {
	int status = SP_ST_DISABLED;
	QString headline, caller, message;
	bool autoAnswer = true;
};

class Dock : public QWidget {
public:
	Dock(sp_dock_action_fn onAction, const QString &footer) : onAction(onAction)
	{
		auto *layout = new QVBoxLayout(this);

		headline = new QLabel(this);
		layout->addWidget(headline);

		auto *callRow = new QHBoxLayout();
		caller = new QLabel(this);
		QFont bold = caller->font();
		bold.setBold(true);
		bold.setPointSizeF(bold.pointSizeF() * 1.2);
		caller->setFont(bold);
		caller->setTextInteractionFlags(Qt::TextSelectableByMouse);
		elapsed = new QLabel(this);
		elapsed->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
		callRow->addWidget(caller, 1);
		callRow->addWidget(elapsed);
		layout->addLayout(callRow);

		message = new QLabel(this);
		message->setWordWrap(true);
		layout->addWidget(message);

		auto *buttons = new QHBoxLayout();
		answer = button(buttons, "Answer", SP_ACT_ANSWER);
		end = button(buttons, "End call", SP_ACT_END);
		retry = button(buttons, "Retry", SP_ACT_RETRY);
		takeBack = button(buttons, "Take the line back", SP_ACT_TAKE_BACK);
		setup = button(buttons, "Open settings", SP_ACT_SETTINGS);
		mute = button(buttons, "Mute caller", SP_ACT_MUTE_TOGGLE);
		buttons->addStretch(1);
		layout->addLayout(buttons);

		auto *warnRow = new QHBoxLayout();
		warning = new QLabel(this);
		warning->setWordWrap(true);
		warning->setStyleSheet(QStringLiteral("color: #d9822b;"));
		fix = new QPushButton(this);
		connect(fix, &QPushButton::clicked, this, [this] { this->onAction(warnAction); });
		warnRow->addWidget(warning, 1);
		warnRow->addWidget(fix);
		layout->addLayout(warnRow);
		setWarning(QString(), SP_WARN_NONE);

		autoAnswer = new QCheckBox("Auto-answer", this);
		connect(autoAnswer, &QCheckBox::toggled, this,
			[this](bool on) { this->onAction(on ? SP_ACT_AUTO_ON : SP_ACT_AUTO_OFF); });
		layout->addWidget(autoAnswer);

		layout->addStretch(1);
		auto *footRow = new QHBoxLayout();
		auto *foot = new QLabel(footer, this);
		foot->setEnabled(false);
		footRow->addWidget(foot, 1);
		auto *settings = new QPushButton("Settings…", this);
		settings->setFlat(true);
		connect(settings, &QPushButton::clicked, this, [this] { this->onAction(SP_ACT_SETTINGS); });
		footRow->addWidget(settings);
		layout->addLayout(footRow);

		blink = new QTimer(this);
		blink->setInterval(500);
		connect(blink, &QTimer::timeout, this, [this] {
			if (state.status == SP_ST_RINGING) {
				// While waiting for a manual Answer, chime every 2 s.
				if (chimeOn && !state.autoAnswer && ++ringTicks % 4 == 0)
					sp_chime_play();
				// Flash the text, not the widget: hiding it would make the
				// rows below (the Answer button) jump up and down.
				blinkOn = !blinkOn;
				caller->setStyleSheet(blinkOn ? QString() : QStringLiteral("color: transparent;"));
			} else if (state.status == SP_ST_ON_AIR) {
				qint64 s = onAirTimer.elapsed() / 1000;
				elapsed->setText(QString("%1:%2").arg(s / 60, 2, 10, QChar('0')).arg(s % 60, 2, 10, QChar('0')));
			}
		});
		blink->start();

		apply(State{});
	}

	void setChime(bool on) { chimeOn = on; }

	void setMuted(bool muted)
	{
		mute->setText(muted ? "Unmute caller" : "Mute caller");
		mute->setStyleSheet(muted ? QStringLiteral("color: #d9822b;") : QString());
	}

	void setWarning(const QString &text, int button)
	{
		warning->setText(text);
		warning->setVisible(!text.isEmpty());
		if (button == SP_WARN_FIX) {
			fix->setText("Fix");
			fix->setToolTip("Untick the caller's return track for Call-In Caller (Advanced Audio Properties)");
			warnAction = SP_ACT_FIX_MIXMINUS;
		} else if (button == SP_WARN_ADD_TO_SCENE) {
			fix->setText("Add to current scene");
			fix->setToolTip("Put Call-In Caller in the scene that's on air");
			warnAction = SP_ACT_ADD_TO_SCENE;
		}
		fix->setVisible(!text.isEmpty() && button != SP_WARN_NONE);
	}

	void apply(const State &next)
	{
		if (next.status == SP_ST_ON_AIR && state.status != SP_ST_ON_AIR)
			onAirTimer.start();
		if (next.status == SP_ST_RINGING && state.status != SP_ST_RINGING) {
			ringTicks = 0;
			if (chimeOn)
				sp_chime_play();
		}
		state = next;

		headline->setText(state.headline);
		bool inCall = state.status == SP_ST_RINGING || state.status == SP_ST_ON_AIR;
		QString prefix = state.status == SP_ST_ON_AIR ? "ON AIR  " : (state.status == SP_ST_RINGING ? "Ringing  " : "");
		caller->setText(prefix + state.caller);
		caller->setVisible(inCall);
		caller->setStyleSheet(QString());
		blinkOn = true;
		elapsed->setVisible(state.status == SP_ST_ON_AIR);
		if (state.status != SP_ST_ON_AIR)
			elapsed->clear();
		message->setText(state.message);
		message->setVisible(!state.message.isEmpty());

		answer->setVisible(state.status == SP_ST_RINGING && !state.autoAnswer);
		end->setVisible(inCall);
		retry->setVisible(state.status == SP_ST_ERROR);
		takeBack->setVisible(state.status == SP_ST_REPLACED);
		setup->setVisible(state.status == SP_ST_SETUP);
		mute->setVisible(inCall);
		autoAnswer->setVisible(state.status != SP_ST_SETUP);

		QSignalBlocker block(autoAnswer);
		autoAnswer->setChecked(state.autoAnswer);
	}

private:
	QPushButton *button(QHBoxLayout *row, const char *text, int action)
	{
		auto *b = new QPushButton(text, this);
		connect(b, &QPushButton::clicked, this, [this, action] { onAction(action); });
		row->addWidget(b);
		return b;
	}

	sp_dock_action_fn onAction;
	State state;
	QLabel *headline, *caller, *elapsed, *message;
	QPushButton *answer, *end, *retry, *takeBack, *setup, *mute, *fix;
	QLabel *warning;
	QCheckBox *autoAnswer;
	QTimer *blink;
	bool blinkOn = true;
	bool chimeOn = true;
	int ringTicks = 0;
	int warnAction = SP_ACT_FIX_MIXMINUS;
	QElapsedTimer onAirTimer;
};

QPointer<Dock> g_dock;
// Receives queued updates on the UI thread. Deleting it drops any still
// queued, so none can run after the plugin is unloaded.
std::atomic<QObject *> g_receiver{nullptr};

} // namespace

void *sp_dock_create(sp_dock_action_fn on_action, const char *footer)
{
	QCoreApplication *app = QCoreApplication::instance();
	bool uiThread = app && QThread::currentThread() == app->thread();
	blog(300, "[obs-softphone] dock: created (app=%s, ui thread=%s)", app ? "yes" : "no", uiThread ? "yes" : "no");
	g_receiver.store(new QObject(app));
	g_dock = new Dock(on_action, QString::fromUtf8(footer));
	return g_dock.data();
}

void sp_dock_update(int status, const char *headline, const char *caller, const char *message, int auto_answer)
{
	QObject *receiver = g_receiver.load();
	if (!receiver)
		return;
	State s;
	s.status = status;
	s.headline = QString::fromUtf8(headline);
	s.caller = QString::fromUtf8(caller);
	s.message = QString::fromUtf8(message);
	s.autoAnswer = auto_answer != 0;
	QMetaObject::invokeMethod(
		receiver,
		[s] {
			blog(300, "[obs-softphone] dock: show status %d (%s)", s.status, g_dock ? "applied" : "no dock");
			if (g_dock)
				g_dock->apply(s);
		},
		Qt::QueuedConnection);
}

void sp_dock_set_warning(const char *text, int button)
{
	QObject *receiver = g_receiver.load();
	if (!receiver)
		return;
	QString t = QString::fromUtf8(text);
	QMetaObject::invokeMethod(
		receiver,
		[t, button] {
			if (g_dock)
				g_dock->setWarning(t, button);
		},
		Qt::QueuedConnection);
}

void sp_dock_set_chime(int on)
{
	QObject *receiver = g_receiver.load();
	if (!receiver)
		return;
	bool c = on != 0;
	QMetaObject::invokeMethod(
		receiver,
		[c] {
			if (g_dock)
				g_dock->setChime(c);
		},
		Qt::QueuedConnection);
}

void sp_dock_set_muted(int muted)
{
	QObject *receiver = g_receiver.load();
	if (!receiver)
		return;
	bool m = muted != 0;
	QMetaObject::invokeMethod(
		receiver,
		[m] {
			if (g_dock)
				g_dock->setMuted(m);
		},
		Qt::QueuedConnection);
}

void sp_dock_shutdown(void)
{
	delete g_receiver.exchange(nullptr);
}
