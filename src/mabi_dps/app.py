"""Small native desktop view over the independently written replay engine."""
import json
from pathlib import Path
import queue
import threading
import time
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from .replay import chart_points, summarize
from .identity import entity_label

BG = '#151b23'
PANEL = '#1c2530'
FG = '#e5edf5'
MUTED = '#9caec1'
BLUE = '#62b5ff'
GREEN = '#62d6aa'
DISPLAY_SOURCE_LIMIT = 16


def number(value):
    return '\u2014' if value is None else f'{value:,.1f}'


class MeterApp:
    def __init__(self, root, *, auto_start=False):
        self.root = root
        self.replay = None
        self.targets = [None]
        self.live_stop = None
        self.live_thread = None
        self.live_events = queue.Queue(maxsize=3)
        self.live_commands = queue.SimpleQueue()
        self.actor_rows = {}
        self.menu_actor_key = None
        self.live_acks=queue.SimpleQueue()
        self.command_id=0
        self.acknowledged_id=0
        root.title('Mabi DPS \u00b7 Independent prototype')
        root.geometry('1040x740')
        root.minsize(720, 570)
        root.configure(bg=BG)
        style = ttk.Style(root)
        style.theme_use('clam')
        style.configure('.', background=BG, foreground=FG, font=('Segoe UI', 10))
        style.configure('TButton', background=PANEL, padding=(12, 7))
        style.map('TButton', background=[('active', '#304156')])
        style.configure('TCombobox', fieldbackground=PANEL, background=PANEL, foreground=FG)
        style.map('TCombobox', fieldbackground=[('readonly', PANEL)],
                  foreground=[('readonly', FG)])
        style.configure('Treeview', background=PANEL, fieldbackground=PANEL,
                        foreground=FG, rowheight=31, borderwidth=0)
        style.configure('Treeview.Heading', background='#283647', foreground=FG, padding=7)
        style.map('Treeview', background=[('selected', '#305073')])
        frame = ttk.Frame(root, padding=22)
        frame.pack(fill='both', expand=True)
        top = ttk.Frame(frame)
        top.pack(fill='x')
        ttk.Label(top, text='MABI / DPS', font=('Segoe UI', 19, 'bold')).pack(side='left')
        self.live_button = ttk.Button(top, text='Start live', command=self.toggle_live)
        self.live_button.pack(side='right', padx=(8, 0))
        ttk.Button(top,text='Reset',command=self.reset_measurement).pack(side='right')
        target_row = ttk.Frame(frame)
        target_row.pack(fill='x', pady=(16, 12))
        ttk.Label(target_row, text='Target').pack(side='left', padx=(0, 10))
        self.target = ttk.Combobox(target_row, values=['All targets'], state='readonly', width=48)
        self.target.current(0)
        self.target.pack(side='left')
        self.target.bind('<<ComboboxSelected>>', lambda event: self.refresh())
        self.pin = tk.BooleanVar(value=False)
        ttk.Checkbutton(target_row, text='Always on top', variable=self.pin,
                        command=lambda: root.attributes('-topmost', self.pin.get())).pack(side='right')
        options=ttk.Frame(frame)
        options.pack(fill='x',pady=(0,10))
        self.encounter_view=tk.BooleanVar(value=True)
        self.include_estimates=tk.BooleanVar(value=True)
        ttk.Checkbutton(options,text='Current encounter',variable=self.encounter_view,command=self.refresh).pack(side='left')
        ttk.Checkbutton(options,text='Include HP estimates',variable=self.include_estimates,command=self.refresh).pack(side='left',padx=12)
        ttk.Label(options,text='End after self idle (s)').pack(side='left',padx=(12,4))
        self.idle_seconds=tk.StringVar(value='15')
        self.idle_control=ttk.Spinbox(options,from_=1,to=300,width=5,textvariable=self.idle_seconds,command=self.change_idle)
        self.idle_control.pack(side='left')
        self.idle_control.bind('<Return>',lambda event:self.change_idle())
        self.encounter_status=tk.StringVar(value='Warming up')
        ttk.Label(options,textvariable=self.encounter_status,foreground=MUTED).pack(side='right')
        cards = ttk.Frame(frame)
        cards.pack(fill='x', pady=(0, 16))
        self.metrics = {}
        for column, title in enumerate(('Damage', 'DPS', 'Duration', 'Hits')):
            card = tk.Frame(cards, bg=PANEL, padx=15, pady=10)
            card.grid(row=0, column=column, sticky='nsew', padx=(0 if column == 0 else 8, 0))
            cards.columnconfigure(column, weight=1, uniform='metric')
            tk.Label(card, text=title.upper(), bg=PANEL, fg=MUTED,
                     font=('Segoe UI', 9)).pack(anchor='w')
            value = tk.StringVar(value='\u2014')
            tk.Label(card, textvariable=value, bg=PANEL, fg=FG,
                     font=('Segoe UI', 20, 'bold')).pack(anchor='w', pady=(2, 0))
            self.metrics[title] = value
        self.source_status = tk.StringVar(value='Top 16 damage sources by DPS')
        ttk.Label(frame, textvariable=self.source_status, foreground=MUTED).pack(anchor='w', pady=(0, 6))
        columns = ('rank', 'actor', 'damage', 'dps', 'share', 'hits', 'critical','estimated')
        table_frame = ttk.Frame(frame)
        table_frame.pack(fill='both', expand=True)
        self.table = ttk.Treeview(table_frame, columns=columns, show='headings', height=6)
        for column, title, width in zip(columns,
                ('#', 'Damage source', 'Damage', 'DPS', 'Share', 'Hits', 'Crit rate','HP estimate'),
                (45, 225, 125, 115, 75, 65, 80,100)):
            self.table.heading(column, text=title)
            self.table.column(column, width=width, anchor='w' if column == 'actor' else 'e')
        self.table.pack(side='left', fill='both', expand=True)
        scrollbar = ttk.Scrollbar(table_frame, orient='vertical', command=self.table.yview)
        scrollbar.pack(side='right', fill='y')
        self.table.configure(yscrollcommand=scrollbar.set)
        self.actor_menu = tk.Menu(root, tearoff=False)
        self.actor_menu.add_command(label='Mark as me',
                                    command=lambda: self.mark_as_me(key=self.menu_actor_key))
        self.actor_menu.add_command(label='Clear my marker', command=lambda: self.mark_as_me(clear=True))
        self.table.bind('<Button-3>', self.show_actor_menu)
        ttk.Label(frame, text='DPS over capture-delivery time   \u2022   Blue: cumulative   '
                  '\u2022   Green: last 5 seconds', foreground=MUTED).pack(anchor='w', pady=(16, 6))
        self.chart = tk.Canvas(frame, height=155, bg=PANEL, highlightthickness=0)
        self.chart.pack(fill='x')
        self.chart.bind('<Configure>', lambda event: self.draw_chart())
        self.status = tk.StringVar(value='Start live capture. Windows will request administrator access for the capture worker.')
        ttk.Label(frame, textvariable=self.status, foreground=MUTED,
                  wraplength=950).pack(anchor='w', pady=(12, 6))
        bottom = ttk.Frame(frame)
        bottom.pack(fill='x')
        ttk.Button(bottom, text='Diagnostics', command=self.show_diagnostics).pack(side='left')
        ttk.Button(bottom,text='Players',command=self.show_players).pack(side='left',padx=5)
        ttk.Button(bottom,text='Recordings',command=self.open_recordings).pack(side='left')
        ttk.Button(bottom, text='Export audit JSON', command=self.export).pack(side='right')
        root.after(100, self.poll)
        root.protocol('WM_DELETE_WINDOW', self.close)
        if auto_start:root.after(300,self.toggle_live)

    def emit_live(self, kind, value=None, status=None):
        while True:
            try:
                self.live_events.put_nowait((kind, value, status))
                return
            except queue.Full:
                try:
                    self.live_events.get_nowait()
                except queue.Empty:
                    pass

    def toggle_live(self):
        if self.live_stop is not None:
            self.live_stop.set()
            self.status.set('Stopping capture worker\u2026')
            return
        self.live_stop = threading.Event()
        self.live_commands = queue.SimpleQueue()
        self.live_acks=queue.SimpleQueue()
        self.command_id=self.acknowledged_id=0
        self.live_button.configure(text='Stop live')
        self.status.set('Waiting for administrator capture worker\u2026')

        def worker():
            from .live import run_live
            from datetime import datetime
            import uuid
            folder=Path(__file__).resolve().parents[2]/'recordings'
            try:
                folder.mkdir(parents=True,exist_ok=True)
                prefix=folder/(datetime.now().strftime('live-%Y%m%d-%H%M%S-')+uuid.uuid4().hex[:6])
                run_live(self.live_stop, lambda view, status: self.emit_live('snapshot', view, status),
                         commands=self.live_commands,on_command=self.live_acks.put,
                         record=prefix.with_suffix('.jsonl'),audit=Path(str(prefix)+'-audit.json'))
            except Exception as exc:
                self.emit_live('error', str(exc))
            finally:
                self.emit_live('ended')

        self.live_thread = threading.Thread(target=worker, daemon=True)
        self.live_thread.start()
        if self.idle_seconds.get()!='15':self.change_idle()

    def send_command(self,kind,**values):
        self.command_id+=1
        self.live_commands.put(dict(id=self.command_id,type=kind,**values))

    def reset_measurement(self):
        if self.replay is None:return
        self.target.current(0)
        self.send_command('reset')

    def change_idle(self):
        try:seconds=float(self.idle_seconds.get())
        except ValueError:return
        self.send_command('idle_seconds',seconds=seconds)

    def open_recordings(self):
        import os
        folder=Path(__file__).resolve().parents[2]/'recordings'
        folder.mkdir(parents=True,exist_ok=True)
        os.startfile(folder)

    def show_players(self):
        window=tk.Toplevel(self.root);window.title('Observed players');window.geometry('660x420')
        ttk.Label(window,text='Received appearances; names stay learned across encounter resets.',padding=10).pack(anchor='w')
        tree=ttk.Treeview(window,columns=('name','actor','realm','self'),show='headings')
        for column,title in (('name','Name'),('actor','Actor'),('realm','Server'),('self','Self')):tree.heading(column,text=title)
        tree.pack(fill='both',expand=True,padx=10,pady=10)
        for row in getattr(self.replay,'observed_players',[]):
            tree.insert('', 'end',values=(row['name'],row['actor'],row['realm'],
                'Yes' if tuple(row['key'])==getattr(self.replay,'self_key',None) else ''))

    def close(self):
        if not hasattr(self, 'closing_deadline'):
            self.closing_deadline = time.monotonic() + 5
        if self.live_stop is not None:
            self.live_stop.set()
        if self.live_thread and self.live_thread.is_alive() and time.monotonic() < self.closing_deadline:
            self.root.after(100, self.close)
        else:
            self.root.destroy()

    def selected_target(self):
        index = self.target.current()
        return self.targets[index] if 0 <= index < len(self.targets) else None

    def poll(self):
        while True:
            try:reply=self.live_acks.get_nowait()
            except queue.Empty:break
            if reply['id']<self.acknowledged_id:continue
            self.acknowledged_id=reply['id']
            self.replay=reply['snapshot']
            self.refresh()
            if not reply['applied']:self.status.set('Command rejected: '+reply['reason'])
            elif reply.get('persistence_error'):self.status.set('Applied; identity cache save failed: '+reply['persistence_error'])
        while True:
            try:
                kind, value, status = self.live_events.get_nowait()
            except queue.Empty:
                break
            if kind == 'snapshot':
                if getattr(value,'last_command',0)<self.acknowledged_id:continue
                previous = self.selected_target()
                self.replay = value
                keys=set(value.summaries)|set(getattr(value,'inferred_summaries',{}))
                self.targets = [None] + sorted(key for key in keys if key is not None)
                self.target.configure(values=self.target_labels())
                self.target.current(self.targets.index(previous) if previous in self.targets else 0)
                self.refresh()
                partial = 'PARTIAL' if value.partial else 'OBSERVED'
                self.status.set(f'{status}  |  {partial}  |  {self.name_status()}')
            elif kind == 'error':
                self.status.set('Live capture failed.')
                messagebox.showerror('Live capture', value)
            elif kind == 'ended':
                self.live_stop = None
                self.live_button.configure(text='Start live')
        self.root.after(100, self.poll)

    def set_replay(self, replay):
        self.replay = replay
        self.targets = [None] + sorted({hit.target_key for hit in replay.hits})
        self.target.configure(values=self.target_labels())
        self.target.current(0)
        self.refresh()

    def actor_label(self, scope, actor, generation=1, *, role='actor'):
        return entity_label(getattr(self.replay, 'actor_names', {}),
                            (scope, actor, generation), role=role,
                            self_key=getattr(self.replay, 'self_key', None))

    def show_actor_menu(self, event):
        row = self.table.identify_row(event.y)
        if row and row in self.actor_rows:
            self.menu_actor_key = self.actor_rows[row]
            self.table.selection_set(row)
            try:
                self.actor_menu.tk_popup(event.x_root, event.y_root)
            finally:
                self.actor_menu.grab_release()

    def mark_as_me(self, *, clear=False, key=None):
        if clear:
            key = None
        elif key is None:
            selected = self.table.selection()
            key = self.actor_rows.get(selected[0]) if selected else None
        if self.replay is None or (key is None and not clear):
            return
        if hasattr(self.replay, 'set_self'):
            self.replay.set_self(key, evidence='user_selection')
        else:
            self.send_command('set_self',key=key)
        self.refresh()

    def target_labels(self):
        from collections import Counter
        labels = [self.actor_label(*key, role='target') for key in self.targets[1:]]
        duplicates = Counter(labels)
        return ['All targets'] + [label + (f' / {key[0]}' if duplicates[label] > 1 else '')
                                  for key, label in zip(self.targets[1:], labels)]

    def name_status(self):
        coverage = self.replay.name_coverage(self.selected_target()) if self.replay else None
        players=len(getattr(self.replay,'observed_players',[])) if self.replay else 0
        self_key=getattr(self.replay,'self_key',None) if self.replay else None
        identity=getattr(self.replay,'actor_names',{}).get(self_key) if self.replay else None
        ready=('Self: '+(identity.name if identity else '<ME>')) if self_key else 'Learning self identity'
        if not coverage or not (coverage['actors']['total'] or coverage['targets']['total']):
            return f'{ready} · {players} player appearances'
        actors, targets = coverage['actors'], coverage['targets']
        status = (f"Names: {actors['resolved']}/{actors['total']} actors, "
                  f"{targets['resolved']}/{targets['total']} targets")
        if actors['resolved'] < actors['total'] or targets['resolved'] < targets['total']:
            status += ' — waiting for appearances'
        return f'{ready} · {players} player appearances · {status}'

    def refresh(self):
        if self.replay is None:
            return
        summary = (self.replay.view_summary(self.selected_target(),encounter=self.encounter_view.get(),estimates=self.include_estimates.get())
                   if hasattr(self.replay,'view_summary') else
                   self.replay.summary(self.selected_target()) if hasattr(self.replay, 'summary')
                   else summarize(self.replay.hits, self.selected_target()))
        self.metrics['Damage'].set(f"{summary['damage']:,}")
        self.metrics['DPS'].set(number(summary['dps']))
        self.metrics['Duration'].set(f"{summary['duration_seconds']:.2f}s")
        self.metrics['Hits'].set(f"{summary['hits']:,}")
        actor_rows = {}
        # Both summaries already rank by damage over the same target duration,
        # hence by DPS. Slice here without re-sorting or altering stored totals.
        displayed = summary['actors'][:DISPLAY_SOURCE_LIMIT]
        self.source_status.set(f"Top {DISPLAY_SOURCE_LIMIT} by DPS \u00b7 "
                               f"Showing {len(displayed)} of {len(summary['actors'])} damage sources"
                               +(f" · {summary['estimated_damage']:,} HP-estimated" if summary.get('estimated_damage') else ''))
        if hasattr(self.replay,'encounter'):
            self.encounter_status.set(self.replay.encounter['status'].capitalize())
        for index, row in enumerate(displayed):
            key = row['scope'], row['actor'], row['actor_generation']
            item = json.dumps(key, separators=(',', ':'))
            values = (index + 1, self.actor_label(*key), f"{row['damage']:,}",
                      number(row['dps']), f"{row['share']:.1%}", row['hits'],
                      f"{row['critical_rate']:.1%}" if row['critical_rate'] is not None else '\u2014',
                      f"{row.get('estimated_damage',0):,}" if row.get('estimated_damage') else '\u2014')
            if self.table.exists(item):
                self.table.item(item, values=values)
                self.table.move(item, '', index)
            else:
                self.table.insert('', index, iid=item, values=values)
            actor_rows[item] = key
        for item in set(self.table.get_children()) - actor_rows.keys():
            self.table.delete(item)
        self.actor_rows = actor_rows
        state = 'PARTIAL \u00b7 inspect diagnostics' if self.replay.partial else 'No replay loss detected'
        frames = getattr(self.replay, 'frame_count', len(self.replay.frames))
        self.status.set(f"{state}  |  {self.replay.segment_count:,} segments / "
                        f"{frames:,} frames  |  {self.name_status()}")
        self.draw_chart()

    def draw_chart(self):
        self.chart.delete('all')
        width, height = self.chart.winfo_width(), self.chart.winfo_height()
        points = (self.replay.view_chart_points(self.selected_target(),encounter=self.encounter_view.get(),estimates=self.include_estimates.get())
                  if hasattr(self.replay,'view_chart_points') else
                  self.replay.chart_points(self.selected_target()) if hasattr(self.replay, 'chart_points')
                  else chart_points(self.replay.hits, self.selected_target())) if self.replay else []
        if not points:
            self.chart.create_text(width / 2, height / 2, text='No damage events', fill=MUTED)
            return
        max_time = max(points[-1][0], 1)
        max_dps = max((value for _, cumulative, rolling in points
                       for value in (cumulative, rolling) if value is not None), default=1) or 1
        left, top, right, bottom = 62, 14, max(80, width - 18), height - 25
        for fraction in (0, 0.5, 1):
            y = bottom - fraction * (bottom - top)
            self.chart.create_line(left, y, right, y, fill='#304156')
            self.chart.create_text(left - 7, y, text=f'{max_dps * fraction:,.0f}', fill=MUTED,
                                   anchor='e', font=('Segoe UI', 8))
        for index, color in ((1, BLUE), (2, GREEN)):
            line = []
            for point in points:
                if point[index] is not None:
                    line.extend((left + point[0] / max_time * (right - left),
                                 bottom - point[index] / max_dps * (bottom - top)))
            if len(line) >= 4:
                self.chart.create_line(*line, fill=color, width=2)
            elif line:
                x, y = line
                self.chart.create_oval(x - 2, y - 2, x + 2, y + 2, fill=color, outline=color)
        self.chart.create_text(left, height - 10, text='0s', anchor='w', fill=MUTED)
        self.chart.create_text(right, height - 10, text=f'{points[-1][0]:.2f}s', anchor='e', fill=MUTED)

    def show_diagnostics(self):
        window = tk.Toplevel(self.root)
        window.title('Capture diagnostics')
        text = tk.Text(window, bg=PANEL, fg=FG, width=100, height=25, wrap='word')
        text.pack(fill='both', expand=True)
        content = ({'issues': dict(self.replay.issues), 'details': self.replay.issue_details,
                    'self_identity': {'key': getattr(self.replay, 'self_key', None),
                                      'evidence': getattr(self.replay, 'self_evidence', None)},
                    'identity_records': dict(self.replay.identity_records),
                    'retained_identity_records': [
                        {key: value for key, value in record.items() if key != 'payload_hex'}
                        for record in self.replay.identity_evidence],
                    'displayed_name_coverage': self.replay.name_coverage(self.selected_target()),
                    'name_acquisition': ('Appearance records enrich existing rows whenever received. '
                        'State refreshes and capture gaps retain cached names in the same game process session. '
                        'Damage packets alone do not contain character names or monster template names. '
                        'Missing player/self records remain unresolved; area re-entry is not a resolution requirement.'),
                    'identity_metadata': (self.replay.lifecycle if hasattr(self.replay, 'lifecycle')
                                          else self.replay.entities.export())}
                   if self.replay else {})
        text.insert('1.0', json.dumps(content, indent=2))
        text.configure(state='disabled')

    def export(self):
        if self.replay is None:
            return
        path = filedialog.asksaveasfilename(defaultextension='.json', initialfile='dps-audit.json',
                                          filetypes=[('Audit JSON', '*.json')])
        if path:
            try:
                Path(path).write_text(json.dumps(self.replay.export(), indent=2), encoding='utf-8')
            except OSError as exc:
                messagebox.showerror('Export failed', str(exc))


def main():
    root = tk.Tk()
    MeterApp(root,auto_start=True)
    root.mainloop()
